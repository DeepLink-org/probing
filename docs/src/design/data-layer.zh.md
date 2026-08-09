# 数据层

Probing 的数据层是一个面向观测数据（指标、采样、trace）的**单进程、抗崩溃、带时间保留的数据面**。
所有生产者都通过同一套自研列式存储 [`probing-memtable`](https://github.com/DeepLink-org/probing)
写入，所有消费者都通过 SQL（DataFusion）读取。它由**两层**构成：

- **热层**（`MEMT`）：固定容量的环形缓冲区，承载实时窗口——常量内存、零分配写入；
- **冷层**（`MEMC`）：不可变、压缩的段文件，用于超出环形窗口的时间保留，按整文件淘汰。

一条 SQL 时间谓词即可同时对两层做剪枝与查询。

## 设计目标

- **资源有界。** 热层环形缓冲永不增长；冷层受字节预算与 TTL 双重约束。
- **抗崩溃。** 写入中途被杀的进程不会暴露半行数据；冷段可从尾部撕裂中通过前向扫描恢复。
- **时间保留。** 滚出热层环形窗口的数据落入冷段，依然可查。
- **一条写路径、一条读路径。** 生产者（server、Python/Torch 扩展）统一写 `probing-memtable`；
  消费者统一走 `probing-core::memtable_sql`。
- **fork 安全。** 在大量 fork 的负载（如 PyTorch DataLoader worker）下依然正确。

## 总体架构

![MEMT 热层、MEMC 冷层与统一查询路径](../assets/architecture/probing-hot-cold-overview.svg)

查询时热层以只读方式 mmap，冷层通过 `SegmentReader` 读取。`HotColdTable` provider 将两者合并为
一次扫描，并对同时存在于两层的 chunk 做去重。

![MEMT 写入、generation 校验与 Arrow 读取](../assets/architecture/probing-memtable-internals.svg)

MEMT 的核心并发关系是单写者对多个 mmap 读者：写者用 generation 和 release/acquire 顺序
发布完整行；读者获取 lease 后复核 chunk 身份，发现环形槽位已被复用时丢弃该批次并重试。

## 热层（MEMT）

### 文件布局

每个 MEMT 缓冲区（堆、共享内存或 mmap 文件）都以 64 字节头部（一个 cache line）开始，随后是
逐列描述符，再是 chunk 数据。

**Header v5（128 字节）：**

| 偏移 | 大小 | 字段 | 说明 |
|---|---|---|---|
| 0 | 4 | `magic` | `0x4D454D54`（`"MEMT"`） |
| 4 | 2 | `version` | 5 |
| 6 | 2 | `header_size` | 128（仅校验） |
| 8 | 2 | `byte_order` | BOM `[0x01,0x02]` |
| 10 | 2 | `ts_col` | 时间戳列索引 + 1（0 = 无） |
| 12 | 4 | `flags` | 特性位（`FLAG_DEDUP` 等） |
| 16 | 4 | `num_cols` | |
| 20 | 4 | `num_chunks` | 环形槽位数 |
| 24 | 4 | `chunk_size` | 每个 chunk 字节数 |
| 28 | 4 | `data_offset` | 64 对齐 |
| 32 | 4 | `write_chunk` | `AtomicU32`——当前环形槽位 |
| 36 | 4 | `refcount` | `AtomicU32` |
| 40 | 4 | `creator_pid` | |
| 44 | 4 | `_pad0` | 对齐填充（v2 中为 `write_lock`） |
| 48 | 8 | `creator_start_time` | 用于发现期的 PID 回收检测 |
| 56 | 8 | `_reserved` | 预留 |
| 64 | 4 | `lease_offset` | reader lease 数组的绝对偏移 |
| 68 | 4 | `lease_slots` | 每个 chunk 的 lease 槽数（当前为 16） |
| 72 | 56 | `_reserved_v5` | 预留 |

字节 0–31 是**冷区**（初始化后不可变），字节 32–63 是**热区**（运行时原子修改），二者分离以避免
伪共享。每个 chunk 以 40 字节的 `ChunkHeader` 开头，携带 `generation` 及逐 chunk 的
`min_ts`/`max_ts`（`AtomicI64`）。每个 reader lease 原子保存进程 PID、本进程引用数和进程
启动时间；回收会阻止新 lease、保留活跃读者，并且只回收已经确认死亡的同一进程实例。

> **v3** 相对 v2：`_pad0` 改为 `ts_col`；移除 `write_lock`（单写者模型）；`ChunkHeader` 新增
> `min_ts`/`max_ts`（24 → 40 字节）。
>
> **v4** 相对 v3：chunk header 的预留字改为原子的 `reader_pins`；查询映射必须允许写入这些
> 元数据，但调用者看到的行数据仍是只读的。
>
> **v5** 相对 v4：不可恢复的聚合 pin 改为每个 chunk 16 个逐进程 lease 槽；reader 崩溃不再
> 永久阻塞环形缓冲区回收。

### 三种后端

同一套 API 支撑三种存储形态：

- **堆内存**——私有 `Vec<u8>`，用于进程内使用；
- **POSIX 共享内存**（`shm_open` + `mmap`）——跨进程、具名、清理时 unlink；
- **文件 mmap**——持久、可发现的文件，位于 `<data_dir>/<pid>/`。SQL 层读取的正是这种。

### 环形缓冲与 generation

写入追加到当前 chunk；当一行放不下时，写入者推进到下一个环形槽位（绕回），同时封存上一个 chunk。
每个槽位携带单调递增的 `generation`（每次环形绕回到该槽位即自增）。读取者按**逻辑顺序（旧 → 新）**
物化 chunk，并在读取后复核 generation——若某 chunk 在读取过程中被回收，则丢弃而非暴露半行数据。

### 单写者模型（无锁）

MEMT 是**单写者**：每个缓冲区恰好一个写者拥有（创建者进程；进程内的写由调用方自行串行化）。缓冲区
内**没有写锁**——写者直接追加行，不在任何锁字上做 CAS 或屏障。读者免锁，与写者之间仅通过逐 chunk
的 `used` / `row_count` 的 `Release` 存储以及 `generation` 复核来协调。

为何安全且足够：

- 生产中每表单写者——Python `ExternalTable` 路径为每个进程写一个文件（命名为 `<data_dir>/<pid>/…`）；
  进程重启即换新 PID、换新文件；
- 读者本就不写锁字，其正确性依赖 `used`/`row_count` 的 `Release`/`Acquire` 次序以及逐 chunk 的
  `generation` 复核；
- 去掉锁还顺带消除了 PID 抢占自旋锁必须防范的 fork 隐患（fork 出的子进程继承了缓存的启动时间，被误
  判为 PID 回收）。

> **冷层（MEMC）** 是另一套并发模型——多个压实写者由 `writer_id` 与段隔离区分——不受 MEMT 单写者
> 模型影响。

### 单写者快路径

由于数据是**单条生成**的，单行提交路径被尽量做轻：

- **每行零分配。** `RowWriter` 流式 API 直接把各字段编码进 ring chunk，不再为每行构造
  `Vec<Value>`。（`push_row(&[Value])` 便捷接口仍可用，但要求调用方先物化一个 value 切片。）
- **无锁，也无每行 `catch_unwind`。** 单写者下既无需加锁，也无需在 panic 时释放锁，因此既不需要逐行
  的 CAS + `Release` 屏障，也不需要 `catch_unwind`/`Drop` 守卫。

读者正确性与写路径无关：行可见性始终依赖 `finish()` 中 `used` / `row_count` 的 `Release` 存储。单线程
`metrics` 实测吞吐（M4，release）：朴素 `push_row` + 自旋锁 ≈ 18.8M 行/s → 流式、免锁 ≈ 29.9M 行/s
（端到端 **+59%**）。

### 时间戳元数据

当 schema 含有名为 `timestamp`（或 `ts`）的 `I64` 列时，`ts_col` 记录其位置，写入路径维护逐
chunk 的 `min_ts`/`max_ts`。这是查询时 chunk 级时间剪枝的基础，并且与冷层的 page/段时间范围在
**结构上完全一致**。

## 冷层（MEMC）

### 目录与文件命名

冷段位于 `<data_dir>/<pid>/cold`——与热层环形文件同处一地、同样按进程隔离，因此冷数据绝不会跨进程
混淆。每个段命名为 `<writer_id>-<seq>.memc`，其中 `writer_id` 是 `(pid, start_time)` 的哈希，
`seq` 是 `ColdStore` 打开时恢复的单调递增序号。

### 段格式

MEMC 要解决的不只是把旧行压缩到磁盘。热层 slot 会被循环复用，进程可能在任意字节处退出，读者又
必须区分“尚未提交”和“已经提交但损坏”。因此 segment 自身承担提交协议和来源身份，而不依赖额外
事务日志。

一个段是一系列 64 对齐的 block。`MCTB` 声明表与 schema，`MCPG` 保存某张表的一列 page，
`MCFT` 是封存时生成的 page 目录。所有完整性校验都使用 **xxh3-64 截断为 32 位**。

**段头部（64 字节）：** `magic`（`"MEMC"`）、`version`（2）、BOM、`flags`（bit 0 = 已封存）、
`writer_pid`、`writer_start`、`created_unix_ms`、`footer_off`（封存前为 0）、段级
`ts_min`/`ts_max`、`page_count`、头部校验和。

![MEMC 的物理段格式、列编码与恢复路径](../assets/architecture/probing-memc-format.svg)

写者依次追加段头、表定义和列 page；所有 payload 与 checksum 完成后才写 footer，最后回写 sealed
header。sealed 位因而是段级提交点。提交前崩溃，读者只承认前向扫描得到的完整 block；提交后读取，
footer、block 或 payload 任一校验失败都视为损坏。这个非对称策略来自因果区别：未封存尾部可能只是
写者尚未完成，已封存段则已经声明自己完整，不能再把损坏解释成正常截断。

page/block 头部携带 `table_id`、`row_count`、`col_count`、`ts_min`/`ts_max`、`payload_len`、
`payload_xxh`，以及对重启去重至关重要的 `source_instance`、`source_gen` 与 `source_chunk`
（该 page 来自哪个热层 MEMT 实例、chunk generation 和索引；`u32::MAX` = 不适用）。头部校验和
覆盖完整的来源身份。

MEMC v2 reader 会明确拒绝 v1 段，因为 v1 缺少实例身份，无法安全参与重启去重。已封存的 v1 文件仍可
被保留策略淘汰；升级时若需保留其历史行，应先归档或转换。

单个段可容纳**多张表**的 page，以 `table_id` 区分。这让文件/目录数量与表数量解耦：成百上千张表
共享同一组段文件。

### 为什么按列编码

Compactor 在离开热路径后把行转置为列，使每类数据按自己的统计特征编码。数值列使用 Pco，尤其适合
单调时间戳；`u8` 保持 `RawFixed`，避免无收益的压缩；字符串和字节串使用带长度的 `RawVarLen`。
编码类型固定在 page header 中，读者不需要猜测。把转置和压缩留给冷层，换来的是 MEMT 仍能保持
逐行、无分配的简单写路径。

### 恢复边界

已封存段通过 footer 直接定位 page，任何完整性失败都会让 SQL 扫描报错；查询不会成功返回一个静默
缺页的冷层结果。未封存段没有作出完整性承诺，因此允许前向扫描并丢弃最后一个不完整 header 或
payload，但一个已经完整写出的 block 若 checksum 错误仍然报错。恢复不尝试拼接半行，也不根据内容
猜测写者意图。

!!! warning "持久性"
    page 不会逐个 `fsync`（仅在封存时 `sync_data`）。`SIGKILL` 可能丢失当前打开段尚未刷盘的尾部
    page。对观测数据可接受，但这是一个明确的取舍。

![MEMC 恢复、保留策略与冷热查询](../assets/architecture/probing-memc-recovery-query.svg)

恢复、TTL/容量保留和 SQL 读取共享同一 segment 格式。已封存段必须通过 footer 与 checksum
校验；未封存段只允许丢弃不完整尾部，不能把中间损坏静默解释成没有数据。

## Compactor（Roller）

`Compactor` 将新封存的热层 chunk 徕出（drain）到冷段。

![Compactor 复核来源 generation、按列编码并在落盘后推进水位](../assets/architecture/probing-memc-lifecycle.svg)

Compactor 不能锁住热层等待压缩，否则冷存储抖动会反向阻塞采集。一次事务先快照 sealed chunk 的
来源身份，再在锁外转置和编码；提交前重新读取 generation。若环形在此期间复用了 slot，当前结果
直接作废。page 完整追加后才推进 `drained_gen`，因此失败只会导致重试，不会错误声称数据已经持久化。

段滚动同时受大小和年龄约束。大小上限控制文件碎片和扫描粒度，年龄上限保证低速率表不会长期停留
在未封存段；显式 flush 使用同一个封存协议。保留策略只删除越过容量或 TTL 的最旧已封存段，始终
保护最新段和可能仍被其他 writer 打开的段。整理、滚动和淘汰由此共享“只对已提交对象做破坏性
决定”的边界。

### 跨重启的精确一次

`drained_gen` 在内存中，因此朴素重启面对持久冷目录时会重新压缩仍驻留在热层环形中的 chunk，产生重复
行。`prime_from_cold()` 在启动时重建高水位：扫描已有冷段，按
`(表, source_instance, source_chunk)` 取 `source_gen` 的最大值，在首次见到某表实例时合并进
`drained_gen`。这样跨重启仍保证**精确一次**，也不会把 generation 从头开始的同名新表误认成旧表。

## 运行时 Owner

`ColdCompactor` 是进程级单例，因为多个后台 owner 会竞争同一热层来源并重复推进水位。后台线程每轮
重新发现 `<data_dir>/<pid>/` 中随运行出现的表，将它们徕出到共享 `ColdStore`，再执行滚动和预算
约束。启动先从冷段重建 watermark，停止时用相同提交协议封存打开段。

发现、段枚举、写入/滚动与 retention I/O 都是可失败操作。worker 会记录 warning，并通过
`CompactorRuntimeStats` 暴露 `error_count` 和带操作上下文的 `last_error`。启动时的 watermark
恢复失败会 fail-closed，因为继续运行可能重复写入冷层；目录或 metadata 读取失败时 retention 也不会
再被当作成功。

它**默认关闭**（opt-in），以避免在每个 fork 出来的 worker 中都启动一个压缩线程。配置通过
`MemTableProbeExtension` 选项面或环境变量下发；server 在引擎初始化时调用
`start_cold_compaction_from_env()`。

## SQL 集成

### Catalog 发现

`<data_dir>/<pid>/` 下的 mmap 文件被暴露为 DataFusion 表，文件名映射到 `(schema, table)`：

- 首个 `.` 分隔 schema 与 table——`acme.actors` → schema `acme`、table `actors`；
- 无 `.` → schema `memtable`（例如 `metrics` → `memtable.metrics`）。

`DynamicMmapCatalog` 将这些动态 schema 与静态 `probe` catalog 合并。形如
`SELECT … FROM probe.memtable.metrics` 的查询经 `MmapFileSchemaProvider::table()` 解析。

### Provider

- **`RingMmapTable`**——热层环形文件之上的惰性 provider。在 `scan()` 时才物化 Arrow batch，并剪掉
  `[min_ts, max_ts]` 无法匹配查询时间谓词的 chunk。
- **`HotColdTable`**——将一个热层环形与其冷段合并为同一张逻辑表（以磁盘 basename 为键，使表名跨
  schema 永不冲突）。这是 catalog 为环形表返回的 provider。

### 三级时间剪枝

一条时间谓词以递增粒度剪枝两层：

1. **段级**——跳过头部 `ts_range` 无法匹配的已封存冷段（无需 mmap）；
2. **page 级**——通过 page 目录跳过范围外的冷 page；
3. **chunk 级**——通过 `min_ts`/`max_ts` 跳过范围外的热 chunk。

热、冷 batch 作为两个分区交给扫描，因此投影、过滤、limit 下推对两层一致生效。

### 热∪冷的精确一次

被压缩的 chunk 在被覆盖前仍存活于热层环形中，朴素的合并会重复计数。`cold_scan` 返回冷 page 来源的
`(source_chunk, source_gen)` 集合；热侧据此**排除**任何 `(索引, 当前 generation)` 落在该集合中的
chunk。每行恰好计数一次，且去重对环形回收免疫（generation 复核会重新验证）。

运行时开关与容量参数属于配置契约，集中在[环境变量 — 数据存储](../reference/env-vars.zh.md#data-storage)，
不在存储架构中重复定义。

## 保证与已知边界

**已保证：**

- 读取无半行数据（generation 复核）；冷层尾部撕裂可恢复；
- 跨层精确一次（查询去重）与跨重启精确一次（`prime_from_cold`）；
- 热层内存有界；冷层字节/TTL 有界；
- 单写者、无锁热路径（MEMT）；读者通过 generation 复核免锁读取。

**已知取舍（P2 待办）：**

- **冷目录按 PID 隔离。** 跨进程隔离干净，但默认不跨重启持久化（新 PID = 新冷目录）。在配置了持久
  冷目录时，`prime_from_cold` 保证重启去重正确。
- **无逐 page `fsync`**——`SIGKILL` 可能丢失打开段尚未刷盘的尾部。
- **无段级 manifest**——多段查询需打开每个段头部做剪枝。
- **Pco level 固定（8）**——未按列自适应。
- **运行时为单进程 per agent**——每个训练进程拥有本地 memtable。跨节点聚合通过 `global`
  联邦 catalog（`global.schema.table`）、HTTP `/apis/cluster/query` 以及
  `probing-core::federation` 中的聚合下推显式完成。

## 测试

数据层附带单元与端到端测试：热层环形的锁/回收/fork 测试（`probing-memtable`）、MEMC 的
格式/恢复/compactor 测试（含带反例的重启去重），以及经运行时 owner 排空、再通过真实 catalog 路径
查询合并结果的 SQL 端到端测试（`probing-core::memtable_sql`）。
