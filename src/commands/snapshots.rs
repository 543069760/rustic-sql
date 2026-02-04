//! `snapshot` subcommand

use crate::{
    Application, RUST_APP,
    helpers::{bold_cell, bytes_size_to_string, table, table_right_from},
    repository::{CliOpenRepo, get_global_grouped_snapshots},
    status_err,
};

use abscissa_core::{Command, Runnable, Shutdown};
use anyhow::Result;
use comfy_table::Cell;
use derive_more::From;
use itertools::Itertools;
// [FIXED] 移除 jiff，使用 std 处理 duration
use std::time::Duration;
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;

use rustic_core::{
    Progress, ProgressBars, SnapshotGroup,
    repofile::{DeleteOption, SnapshotFile},
};

#[cfg(feature = "tui")]
use crate::commands::tui;

/// `snapshot` subcommand
#[derive(clap::Parser, Command, Debug)]
pub(crate) struct SnapshotCmd {
    #[clap(value_name = "ID")]
    ids: Vec<String>,

    #[arg(long)]
    long: bool,

    #[clap(long, conflicts_with = "long")]
    json: bool,

    #[clap(long, conflicts_with_all = &["long", "json", "all"])]
    sql: bool,

    #[clap(long, value_name = "PATH", requires = "sql")]
    sql_output: Option<PathBuf>,

    #[clap(long, conflicts_with_all = &["long", "json", "sql"])]
    all: bool,

    #[cfg(feature = "tui")]
    #[clap(long, short)]
    pub interactive: bool,
}

impl Runnable for SnapshotCmd {
    fn run(&self) {
        if let Err(err) = RUSTIC_APP
            .config()
            .repository
            .run_open(|repo| self.inner_run(repo))
        {
            status_err!("{}", err);
            RUSTIC_APP.shutdown(Shutdown::Crash);
        };
    }
}

impl SnapshotCmd {
    fn inner_run(&self, repo: CliOpenRepo) -> Result<()> {
        #[cfg(feature = "tui")]
        if self.interactive {
            return tui::run(|progress| {
                let config = RUSTIC_APP.config();
                config
                    .repository
                    .run_indexed_with_progress(progress.clone(), |repo| {
                        let p = progress.progress_spinner("starting rustic in interactive mode...");
                        p.finish();
                        let snapshots = tui::Snapshots::new(
                            &repo,
                            config.snapshot_filter.clone(),
                            config.global.group_by.unwrap_or_default(),
                        )?;
                        tui::run_app(progress.terminal, snapshots)
                    })
            });
        }

        let groups = get_global_grouped_snapshots(&repo, &self.ids)?;

        if self.json {
            #[derive(Serialize, From)]
            struct SnapshotsGroup {
                group_key: SnapshotGroup,
                snapshots: Vec<SnapshotFile>,
            }
            let groups: Vec<SnapshotsGroup> = groups.into_iter().map(|g| g.into()).collect();
            let mut stdout = std::io::stdout();
            if groups.len() == 1 && groups[0].group_key.is_empty() {
                serde_json::to_writer_pretty(&mut stdout, &groups[0].snapshots)?;
            } else {
                serde_json::to_writer_pretty(&mut stdout, &groups)?;
            }
            return Ok(());
        }

        if self.sql {
            if let Some(output_path) = &self.sql_output {
                let mut file = std::fs::File::create(output_path)?;
                write_snapshots_as_sql(&groups, &mut file)?;
            } else {
                let mut stdout = std::io::stdout();
                write_snapshots_as_sql(&groups, &mut stdout)?;
            }
            return Ok(());
        }

        let mut total_count = 0;
        for (group_key, snapshots) in groups {
            if !group_key.is_empty() {
                println!("\nsnapshots for {group_key}");
            }
            total_count += snapshots.len();
            print_snapshots(snapshots, self.long, self.all);
        }
        println!();
        println!("total: {total_count} snapshot(s)");

        Ok(())
    }
}

fn sql_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "''")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace('\0', "\\0")
}

fn sql_quote_str(opt: &Option<String>) -> String {
    match opt {
        Some(s) => format!("'{}'", sql_escape(s)),
        None => "NULL".to_string(),
    }
}

pub fn write_snapshots_as_sql<W: Write>(
    groups: &[(SnapshotGroup, Vec<SnapshotFile>)],
    writer: &mut W,
) -> Result<()> {
    writeln!(writer, "PRAGMA foreign_keys = ON;")?;
    writeln!(writer, "BEGIN TRANSACTION;")?;

    writeln!(writer,
             "CREATE TABLE IF NOT EXISTS snapshots (
            id TEXT PRIMARY KEY NOT NULL,
            time TEXT NOT NULL,
            program_version TEXT NOT NULL,
            parent TEXT,
            tree TEXT NOT NULL,
            hostname TEXT NOT NULL,
            username TEXT NOT NULL,
            uid INTEGER NOT NULL,
            gid INTEGER NOT NULL,
            original TEXT,
            label TEXT NOT NULL,
            description TEXT,
            delete_condition TEXT NOT NULL DEFAULT 'not set'
        );")?;

    writeln!(writer, "CREATE TABLE IF NOT EXISTS snapshot_paths (snapshot_id TEXT NOT NULL, path TEXT NOT NULL, PRIMARY KEY (snapshot_id, path), FOREIGN KEY(snapshot_id) REFERENCES snapshots(id) ON DELETE CASCADE) WITHOUT ROWID;")?;
    writeln!(writer, "CREATE TABLE IF NOT EXISTS snapshot_tags (snapshot_id TEXT NOT NULL, tag TEXT NOT NULL, PRIMARY KEY (snapshot_id, tag), FOREIGN KEY(snapshot_id) REFERENCES snapshots(id) ON DELETE CASCADE) WITHOUT ROWID;")?;

    writeln!(writer,
             "CREATE TABLE IF NOT EXISTS snapshot_summaries (
            snapshot_id TEXT PRIMARY KEY NOT NULL,
            files_new INTEGER NOT NULL,
            files_changed INTEGER NOT NULL,
            files_unmodified INTEGER NOT NULL,
            total_files_processed INTEGER NOT NULL,
            total_bytes_processed INTEGER NOT NULL,
            dirs_new INTEGER NOT NULL,
            dirs_changed INTEGER NOT NULL,
            dirs_unmodified INTEGER NOT NULL,
            total_dirs_processed INTEGER NOT NULL,
            total_dirsize_processed INTEGER NOT NULL,
            data_blobs INTEGER NOT NULL,
            tree_blobs INTEGER NOT NULL,
            data_added INTEGER NOT NULL,
            data_added_packed INTEGER NOT NULL,
            data_added_files INTEGER NOT NULL,
            data_added_files_packed INTEGER NOT NULL,
            data_added_trees INTEGER NOT NULL,
            data_added_trees_packed INTEGER NOT NULL,
            command TEXT NOT NULL,
            backup_start TEXT NOT NULL,
            backup_end TEXT NOT NULL,
            backup_duration REAL NOT NULL,
            total_duration REAL NOT NULL,
            FOREIGN KEY(snapshot_id) REFERENCES snapshots(id) ON DELETE CASCADE
        );")?;

    for (_, snapshots) in groups {
        for snap in snapshots {
            let id = snap.id.to_hex().to_string();
            writeln!(writer,
                     "INSERT OR REPLACE INTO snapshots VALUES ('{}', '{}', '{}', {}, '{}', '{}', '{}', {}, {}, {}, '{}', {}, '{}');",
                     id,
                     snap.time.to_string(),
                     sql_escape(&snap.program_version),
                     snap.parent.map(|p| format!("'{}'", p.to_hex())).unwrap_or_else(|| "NULL".into()),
                     snap.tree.to_hex(),
                     sql_escape(&snap.hostname),
                     sql_escape(&snap.username),
                     snap.uid,
                     snap.gid,
                     snap.original.map(|o| format!("'{}'", o.to_hex())).unwrap_or_else(|| "NULL".into()),
                     sql_escape(&snap.label),
                     sql_quote_str(&snap.description),
                     match &snap.delete {
                         DeleteOption::NotSet => "not set",
                         DeleteOption::Never => "never",
                         DeleteOption::After(_) => "after",
                     }
            )?;

            for path in &snap.paths {
                writeln!(writer, "INSERT OR IGNORE INTO snapshot_paths VALUES ('{}', '{}');", id, sql_escape(path))?;
            }
            for tag in &snap.tags {
                writeln!(writer, "INSERT OR IGNORE INTO snapshot_tags VALUES ('{}', '{}');", id, sql_escape(tag))?;
            }

            if let Some(s) = &snap.summary {
                writeln!(writer,
                         "INSERT OR REPLACE INTO snapshot_summaries VALUES ('{}', {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, '{}', '{}', '{}', {}, {});",
                         id,
                         s.files_new, s.files_changed, s.files_unmodified,
                         s.total_files_processed, s.total_bytes_processed,
                         s.dirs_new, s.dirs_changed, s.dirs_unmodified,
                         s.total_dirs_processed, s.total_dirsize_processed,
                         s.data_blobs, s.tree_blobs,
                         s.data_added, s.data_added_packed,
                         s.data_added_files, s.data_added_files_packed,
                         s.data_added_trees, s.data_added_trees_packed,
                         sql_escape(&s.command),
                         s.backup_start.to_string(),
                         s.backup_end.to_string(),
                         s.backup_duration,
                         s.total_duration
                )?;
            }
        }
    }

    writeln!(writer,
             "CREATE VIEW IF NOT EXISTS v_snapshots_full AS
        SELECT
            s.*,
            (SELECT group_concat(tag, char(31)) FROM snapshot_tags WHERE snapshot_id = s.id) AS tags_flat,
            (SELECT group_concat(path, char(31)) FROM snapshot_paths WHERE snapshot_id = s.id) AS paths_flat,
            sum.files_new, sum.files_changed, sum.files_unmodified,
            sum.total_files_processed, sum.total_bytes_processed,
            sum.dirs_new, sum.dirs_changed, sum.dirs_unmodified,
            sum.total_dirs_processed, sum.total_dirsize_processed,
            sum.data_blobs, sum.tree_blobs,
            sum.data_added, sum.data_added_packed,
            sum.data_added_files, sum.data_added_files_packed,
            sum.data_added_trees, sum.data_added_trees_packed,
            sum.command, sum.backup_start, sum.backup_end,
            sum.backup_duration, sum.total_duration
        FROM snapshots s
        LEFT JOIN snapshot_summaries sum ON s.id = sum.snapshot_id;")?;

    writeln!(writer, "COMMIT;")?;
    Ok(())
}

pub fn print_snapshots(snapshots: Vec<SnapshotFile>, long: bool, all: bool) {
    let count = snapshots.len();
    if long {
        for snap in snapshots {
            let mut table = table();
            let add_entry = |title: &str, value: String| {
                _ = table.add_row([bold_cell(title), Cell::new(value)]);
            };
            fill_table(&snap, add_entry);
            println!("{table}");
            println!();
        }
    } else {
        let mut table = table_right_from(
            6,
            [
                "ID", "Time", "Host", "Label", "Tags", "Paths", "Files", "Dirs", "Size",
            ],
        );

        if all {
            _ = table.add_rows(snapshots.into_iter().map(|sn| snap_to_table(&sn, 0)));
        } else {
            _ = table.add_rows(
                snapshots
                    .into_iter()
                    .chunk_by(|sn| sn.tree)
                    .into_iter()
                    .map(|(_, mut g)| snap_to_table(&g.next().unwrap(), g.count())),
            );
        }
        println!("{table}");
    }
    println!("{count} snapshot(s)");
}

pub fn snap_to_table(sn: &SnapshotFile, count: usize) -> [String; 9] {
    let tags = sn.tags.formatln();
    let paths = sn.paths.formatln();
    // [FIXED] v0.10.3 兼容的时间格式化
    let time = sn.time.format("%Y-%m-%d %H:%M:%S").to_string();
    let (files, dirs, size) = sn.summary.as_ref().map_or_else(
        || ("?".to_string(), "?".to_string(), "?".to_string()),
        |s| {
            (
                s.total_files_processed.to_string(),
                s.total_dirs_processed.to_string(),
                bytes_size_to_string(s.total_bytes_processed),
            )
        },
    );
    let id = match count {
        0 => format!("{}", sn.id),
        count => format!("{} (+{})", sn.id, count),
    };
    [
        id,
        time,
        sn.hostname.clone(),
        sn.label.clone(),
        tags,
        paths,
        files,
        dirs,
        size,
    ]
}

pub fn fill_table(snap: &SnapshotFile, mut add_entry: impl FnMut(&str, String)) {
    add_entry("Snapshot", snap.id.to_hex());
    if let Some(original) = snap.original {
        if original != snap.id {
            add_entry("Original ID", original.to_hex());
        }
    }
    // [FIXED] 兼容格式
    add_entry("Time", snap.time.format("%Y-%m-%d %H:%M:%S").to_string());
    add_entry("Generated by", snap.program_version.clone());
    add_entry("Host", snap.hostname.clone());
    add_entry("Label", snap.label.clone());
    add_entry("Tags", snap.tags.formatln());
    let delete = match &snap.delete {
        DeleteOption::NotSet => "not set".to_string(),
        DeleteOption::Never => "never".to_string(),
        DeleteOption::After(t) => format!("after {}", t.format("%Y-%m-%d %H:%M:%S")),
    };
    add_entry("Delete", delete);
    add_entry("Paths", snap.paths.formatln());
    let parent = snap.parent.map_or_else(
        || "no parent snapshot".to_string(),
        |p| p.to_hex(),
    );
    add_entry("Parent", parent);
    if let Some(ref summary) = snap.summary {
        add_entry("", String::new());
        add_entry("Command", summary.command.clone());

        let source = format!(
            "files: {} / dirs: {} / size: {}",
            summary.total_files_processed,
            summary.total_dirs_processed,
            bytes_size_to_string(summary.total_bytes_processed)
        );
        add_entry("Source", source);
        add_entry("", String::new());

        let files = format!(
            "new: {:>10} / changed: {:>10} / unchanged: {:>10}",
            summary.files_new, summary.files_changed, summary.files_unmodified,
        );
        add_entry("Files", files);

        let trees = format!(
            "new: {:>10} / changed: {:>10} / unchanged: {:>10}",
            summary.dirs_new, summary.dirs_changed, summary.dirs_unmodified,
        );
        add_entry("Dirs", trees);
        add_entry("", String::new());

        let written = format!(
            "data:  {:>10} blobs / raw: {:>10} / packed: {:>10}\n\
            tree:  {:>10} blobs / raw: {:>10} / packed: {:>10}\n\
            total: {:>10} blobs / raw: {:>10} / packed: {:>10}",
            summary.data_blobs,
            bytes_size_to_string(summary.data_added_files),
            bytes_size_to_string(summary.data_added_files_packed),
            summary.tree_blobs,
            bytes_size_to_string(summary.data_added_trees),
            bytes_size_to_string(summary.data_added_trees_packed),
            summary.tree_blobs + summary.data_blobs,
            bytes_size_to_string(summary.data_added),
            bytes_size_to_string(summary.data_added_packed),
        );
        add_entry("Added to repo", written);

        // [FIXED] 格式化持续时间
        let duration = format!(
            "backup start: {} / backup end: {} / backup duration: {:.2}s\n\
            total duration: {:.2}s",
            summary.backup_start.format("%Y-%m-%d %H:%M:%S"),
            summary.backup_end.format("%Y-%m-%d %H:%M:%S"),
            summary.backup_duration,
            summary.total_duration,
        );
        add_entry("Duration", duration);
    }
    if let Some(ref description) = snap.description {
        add_entry("Description", description.clone());
    }
}