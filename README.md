# Rustic SQL 导出功能文档

## 📌 概述

`rustic` 现在支持将快照元数据直接导出为 **SQLite 兼容的 SQL 格式**。

该功能专为以下场景设计：

* **高性能需求**：当仓库规模过大（数千个快照），传统的 JSON 输出解析效率低下时。
* **移动端集成**：需要与使用 **Android Room** 的移动应用进行无缝元数据同步时。
* **复杂分析**：利用 SQL 强大的查询能力进行备份审计和增长趋势分析。

---

## ⚙️ 技术实现

SQL 导出功能生成规范化的数据库架构，包含以下核心表结构：

* **`snapshots`**: 主表，包含快照元数据（ID、时间、主机名等）。
* **`snapshot_paths`**: 1:N 关系，存储每个快照对应的备份路径。
* **`snapshot_tags`**: 1:N 关系，存储快照标签。
* **`snapshot_summaries`**: 1:1 关系，存储详细的备份统计信息（文件数、字节数、时长等）。

### 核心特性

* **原子性**: 整个导出过程包装在单个 SQL 事务中。
* **完整性**: 包含外键约束，支持 `ON DELETE CASCADE` 级联删除。
* **高性能**: 针对常用查询列（时间、主机名）预建索引。
* **空间优化**: 连接表使用 `WITHOUT ROWID` 优化。
* **移动端友好**: 完全兼容 Android Room 实体的映射要求。

---

## 🚀 使用方法

### 1. 基础用法

```bash
# 输出 SQL 到终端
rustic snapshots --sql

# 重定向到文件
rustic snapshots --sql > snapshots.sql

# 使用内置参数直接输出到文件
rustic snapshots --sql --sql-output /path/to/snapshots.sql

```

### 2. 结合过滤器

您可以利用 `rustic` 强大的过滤功能，只导出特定的元数据：

```bash
# 导出指定主机的快照
rustic snapshots --sql --filter-host myserver

# 导出特定日期范围
rustic snapshots --sql --filter-after "2024-01-01" --filter-before "2024-12-31"

# 按标签过滤
rustic snapshots --sql --filter-tags backup,production

```

### 3. 导入 SQLite 进行分析

```bash
# 创建数据库并导入数据
sqlite3 snapshots.db < snapshots.sql

# 示例：查询各主机的备份总数
sqlite3 snapshots.db "SELECT hostname, COUNT(*) FROM snapshots GROUP BY hostname;"

```

---

## 📱 Android Room 集成

SQL 架构经过专门设计，可直接映射到 Kotlin 实体类。

<details>
<summary>点击查看 Entity 示例代码</summary>

```kotlin
@Entity(tableName = "snapshots")  
data class Snapshot(  
    @PrimaryKey val id: String,  
    val time: String,  
    @ColumnInfo(name = "program_version") val programVersion: String,  
    val parent: String?,  
    val tree: String,  
    val hostname: String,  
    val username: String,  
    val uid: Int,  
    val gid: Int,  
    val original: String?,  
    val label: String,  
    val description: String?,  
    @ColumnInfo(name = "delete_condition") val deleteCondition: String  
)  

@Entity(  
    tableName = "snapshot_paths",  
    primaryKeys = ["snapshot_id", "path"],  
    foreignKeys = [ForeignKey(  
        entity = Snapshot::class,  
        parentColumns = ["id"],  
        childColumns = ["snapshot_id"],  
        onDelete = ForeignKey.CASCADE  
    )]  
)  
data class SnapshotPath(  
    @ColumnInfo(name = "snapshot_id") val snapshotId: String,  
    val path: String  
)

```

</details>

---

## 📊 常见 SQL 查询场景

### 查找特定主机最近一个月的备份详情

```sql
SELECT s.id, s.time, s.hostname,   
       GROUP_CONCAT(p.path) as paths,  
       GROUP_CONCAT(t.tag) as tags  
FROM snapshots s  
LEFT JOIN snapshot_paths p ON s.id = p.snapshot_id  
LEFT JOIN snapshot_tags t ON s.id = t.snapshot_id  
WHERE s.hostname = 'production-server'  
  AND s.time > datetime('now', '-1 month')  
GROUP BY s.id  
ORDER BY s.time DESC;

```

### 查看存储增长趋势 (GB)

```sql
SELECT DATE(time) as date,  
       SUM(total_bytes_processed) / 1024 / 1024 / 1024 as total_gb  
FROM snapshots s  
JOIN snapshot_summaries ss ON s.id = ss.snapshot_id  
GROUP BY DATE(time)  
ORDER BY date;

```

---

## ⚡ 性能与对比

| 特性 | JSON 输出 | SQL 导出 |
| --- | --- | --- |
| **文件大小** | 较大 (冗余嵌套结构) | **较小 (规范化存储)** |
| **查询速度** | 慢 (需全量解析) | **极快 (索引查询)** |
| **内存消耗** | 高 (需加载整个对象) | **低 (流式写入/读)** |
| **工具支持** | jq, JSON 解析器 | **SQLite, BI 工具, Room** |
| **人类可读性** | 较好 | 需 SQL 知识 |

---

## 🛠️ 故障排除

### GLIBC 版本报错

如果在旧版 Linux 上遇到 `GLIBC_X.XX not found`，请下载 **musl** 静态链接版本：

```bash
https://github.com/543069760/rustic-sql/actions/runs/21696060591
```

### 输出被截断

如果直接在终端查看导致数据丢失，请务必使用文件输出参数：

```bash
rustic snapshots --sql --sql-output complete.sql

```

---

> ℹ️ **更多功能**: 其他 `rustic` 核心功能与原版保持一致。
> 🔗 **参考文档**: [Rustic 官方 GitHub](https://github.com/rustic-rs/rustic)

---
