# probing-memtable

`probing-memtable` 是一个面向高吞吐写入和低开销读取的内存表。

它把数据写入一块自描述的连续内存中，并按固定大小的 chunk 组织成 ring buffer。每一行可以包含固定宽度列，也可以包含字符串或二进制这类变长列。

这个库的目标不是做一个通用数据库，而是提供一个：

- 足够快的内存数据面
- 足够简单的写入和扫描接口
- 可以用于共享内存、外部 buffer 或嵌入式数据通道的基础组件

## 它解决什么问题

当你需要下面这类能力时，这个库比较合适：

- 持续向内存里追加结构化记录
- 按 chunk 顺序扫描最近写入的数据
- 控制内存占用，不希望无限增长
- 希望读路径尽量轻量，不依赖复杂运行时
- 希望底层 buffer 可以被复用、传递或映射

它特别适合日志、事件、指标、trace 片段、节点内共享缓冲区这类场景。

## 它不是什么

它不是一个完整数据库，也不提供：

- SQL
- 索引
- 查询优化
- 持久化存储
- 多写者事务语义

它更像一个简单、可控、性能优先的 memtable / ring buffer 数据结构。

## 核心概念

### Schema

`Schema` 定义列名和列类型。

### MemTable

`MemTable` 是默认入口。它持有一块内部 buffer，并提供最简单的写入和读取方式。

### Chunk

底层内存被分成多个固定大小的 chunk。写满当前 chunk 后，会自动推进到下一个 chunk。到末尾后会回绕，形成 ring buffer。

### Row 和 RowCursor

`rows(chunk)` 返回某个 chunk 中的行。

推荐使用 `RowCursor` 顺序读取一行中的各列。这是最简单、最稳定的读取方式。

## 最小使用示例

```rust
use probing_memtable::{DType, MemTable, Schema, Value};

let schema = Schema::new()
    .col("ts", DType::I64)
    .col("msg", DType::Str);

let mut table = MemTable::new(&schema, 4096, 4);

table.push_row(&[Value::I64(1), Value::Str("hello")]);
table.push_row(&[Value::I64(2), Value::Str("world")]);

for row in table.rows(0) {
    let mut c = row.cursor();
    let ts = c.next_i64();
    let msg = c.next_str();
    println!("{ts} {msg}");
}
```

## 推荐的默认用法

如果你是第一次使用这个库，建议只记住下面这条主路径：

1. 定义 `Schema`
2. 创建 `MemTable`
3. 用 `push_row()` 写入
4. 用 `rows()` + `RowCursor` 读取

这条路径最容易理解，也最容易被别人接受。

## 公开 API 分层

### 默认接口

下面这些接口是大多数场景需要的：

- `Schema`
- `DType`
- `Value`
- `MemTable`
- `Row`
- `RowCursor`

### 高级接口

下面这些接口主要给性能敏感或共享 buffer 场景使用：

- `RowWriter`
- `MemTableWriter`
- `CachedReader`
- `MemTableView`
- `validate_buf`
- `acquire_ref` / `release_ref`

如果你只是想把数据写进去再读出来，可以先忽略它们。

## 写入路径分层

从安全到高性能，有三条写入路径：

| 接口 | 校验 | 速度 | 适用场景 |
|---|---|---|---|
| `push_row()` | 完整 schema 校验 | 基准 | 默认路径，最安全 |
| `push_row_unchecked()` | 无校验 | ~2× | 确认 schema 正确后的热路径 |
| `row_writer()` | 无校验，流式写入 | ~3× | 极致性能，调用方按 schema 顺序写 |

如果你不确定该用哪个，优先用 `push_row()`。

## 关于 dedup

`MemTableWriter` 开启 dedup 模式后，会在当前写入 chunk 内，对重复的字符串或二进制值做按列去重，以减少内存占用。

这是一个空间优化能力，不是默认入口。只有当你确认数据里存在大量重复字符串/bytes 时，再考虑使用它。

## 并发模型

这个库的并发模型是刻意保持简单的：

- 写入路径串行化
- 读取路径尽量轻量
- chunk 回绕时用 generation 检测陈旧读取

如果你把它当成一个高性能的单写者内存表，会比把它当成通用并发数据库更符合它的设计初衷。

## 何时使用它

适合：

- 高频追加写
- 顺序扫描
- 固定内存预算
- 节点内缓冲区
- 共享内存或外部 buffer 封装

不适合：

- 复杂查询
- 多写者强一致
- 长生命周期随机访问数据集
- 想直接替代数据库的场景

## 一句话总结

`probing-memtable` 是一个简单、可控、性能优先的 ring-buffer memtable，用来把结构化记录持续写入内存，并以低开销方式顺序读取。
