## Zebin 设计文档

本文档描述当前代码库的真实实现，不再保留旧版 `VTable` 术语。现阶段的核心抽象是 `LayoutDescriptor`、`LayoutDirectory` 和 `schema_id`。

### 1. 设计目标

- 零拷贝读取 archived 数据。
- 允许结构体字段按 schema 进行演进。
- 通过显式校验布局描述，避免仅靠裸指针或隐式偏移推断对象结构。
- 保持实现简单，优先服务当前库，而不是兼容历史设计。

### 2. 核心概念

- `Archive`：类型级别契约，描述原始类型、archived 类型和 resolver。
- `Serialize`：把值写入字节流，并在需要时注册布局。
- `Validate`：对 archived 数据做安全校验。
- `RelPtr<T>`：相对指针，仅用于 `String` 和 `Vec` 这类引用型字段。
- `LayoutField`：单个字段的布局条目，包含 `field_id` 和 `offset`。
- `LayoutDescriptor`：构建期的布局集合，用于去重。
- `LayoutDirectory`：读取期的布局目录，用 `schema_id` 查找布局。
- `ArchiveView<T>`：decode 后返回的安全视图，持有原始字节、头部和根对象借用。

### 3. 当前二进制格式

当前 archive 头部固定为 12 字节，格式如下：

- `0..2`：magic，固定为 `ZB`
- `2`：版本号，当前为 `1`
- `3`：flags，当前写死为 `0`
- `4..8`：`layout_offset`
- `8..12`：`root_offset`

`layout_offset` 指向文件尾部的布局目录，`root_offset` 指向根对象。

### 4. 布局目录格式

布局目录位于 archive 尾部，结构如下：

- `u32 num_layouts`
- `u32[num_layouts] layout_offsets`
- 每个 layout entry：
  - `u32 schema_id`
  - `u16 field_count`
  - `u16 reserved`
  - `field_count` 个字段记录

字段记录格式：

- `u16 field_id`
- `u16 offset`

当前实现对 `schema_id` 的约束是顺序分配、连续编号。也就是说，第 0 个注册的布局对应 `schema_id = 0`，第 1 个对应 `schema_id = 1`，以此类推。

读取侧会再次校验：

- 目录头部和偏移表没有越界
- 目录中的 `schema_id` 必须和表索引一致
- `LayoutDirectory::lookup(schema_id)` 取得的条目必须和存储的 `schema_id` 一致

### 5. 可演进对象模型

如果一个 Rust 结构体的字段带有 `#[zebin(id = ...)]`，则它被视为可演进对象。

可演进对象有两个特点：

- archived 结构体头部会显式携带 `schema_id: u32`
- resolver 也会携带 `schema_id: u32`

这意味着对象本身在落盘时就知道自己属于哪个布局，而不是依赖外部约定去猜测。

如果结构体没有任何字段 ID 标注，则它走稳定布局，不携带 `schema_id`。

### 6. 序列化流程

`encode()` 的实际流程是一个两阶段策略：

1. 先用 `MeasureEncoder` 进行测量，计算总长度并收集所有布局。
2. 再用 `SliceEncoder` 真正写入到目标缓冲区。

`ArchiveWriter` 的写入阶段是分段状态机：

- `Header`
- `Body`
- `RootAlign`
- `Root`
- `Layout`
- `Done`

`ArchiveWriter::write()` 可以被多次调用，支持 chunked 输出。内部会检查：

- 头部是否完整写出
- body 阶段是否继续推进
- root 的对齐和实际位置是否和计划一致
- root 写出后，运行期收集到的布局是否和测量阶段一致
- 最后写出布局目录

`encode()`、`encode_into()` 和 `encode_chunked()` 只是对这个 writer 的不同包装。

### 7. 读取与校验流程

`decode()` 和 `validate()` 会先做结构级校验：

- 校验魔数和版本号
- 校验 root 是否在边界内
- 校验 root 对齐
- 校验布局目录本身是否完整
- 校验布局目录不会和 root 区域重叠

之后进入递归验证：

- `Validator` 维护当前字节切片、布局目录和递归深度
- 如果对象带有 `schema_id`，验证器会先通过 `LayoutDirectory` 找到对应布局
- 再比对字段 `field_id -> offset` 是否和代码生成结果一致
- 最后递归验证各字段自身

### 8. 相对指针

`RelPtr<T>` 只负责“从自身位置到目标位置”的偏移，不负责布局语义。

当前实现里：

- `String` 的 archived 表示使用 `RelPtr<u8> + len`
- `Vec<T>` 的 archived 表示使用 `RelPtr<T> + len`
- 空字符串和空向量会使用 null relative pointer

### 9. 过程宏行为

`ZebinArchive` 和 `ZebinSerialize` 现在围绕布局语义工作。

当前规则是：

- 只支持具名字段结构体
- 如果任意字段使用了 `#[zebin(id = ...)]`, 则该结构体会被视为可演进类型
- 一旦进入可演进模式，所有字段都必须带 `id`
- 可演进类型会自动生成 `schema_id` 字段和按 schema 查询的访问器

`ZebinSerialize` 会为结构体生成一个独立的 `State<'a>`：

- 每个字段有自己的子状态
- 每个字段有自己的 resolver 槽位
- 可演进类型的 `schema_id` 在状态中是 `Option<u32>`，首次注册布局时写入

`ZebinArchive` 会生成：

- `Archived{Name}`
- `{Name}Resolver`
- `impl Archive for {Name}`
- `impl Validate<Validator<'_>> for Archived{Name}`

对于可演进类型，还会额外生成 `Archived{Name}` 的 unsafe 按字段访问器。访问器会：

- 读取 archive header
- 通过 `LayoutDirectory::lookup(schema_id)` 找到布局
- 用 `field_id` 定位字段偏移
- 返回 archived 字段引用

### 10. 模块结构

当前核心模块命名如下：

- `core/rel_ptr.rs`
- `core/schema.rs`
- `core/validator.rs`

其中 `schema.rs` 是当前布局语义的承载模块，提供 `LayoutField`、`LayoutDescriptor`、`LayoutView` 和 `LayoutDirectory`。

### 11. 现阶段边界

- 当前没有引入 archive 版本升级机制，版本号仍固定为 `1`。
- 当前没有多语言 IDL。
- 当前没有 varint、bit packing 或 SIMD 专用编码路径。
- 当前没有把布局描述暴露成原始指针，读取侧只使用借用视图。

### 12. 未来可扩展点

- 为 `schema_id` 引入更灵活的版本协商。
- 支持字段默认值与可选字段语义。
- 支持更细粒度的布局兼容策略，例如字段缺失时的降级读取。
- 如果未来需要跨版本演进，可以在不改 archive 头版本的前提下扩展布局目录字段。
