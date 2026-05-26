use crate::{
    error::ZebinError,
    io::{ByteSink, Encoder},
    schema::FieldEncoding,
};

/// Helper encoder to serialize a field entry reentrantly.
#[derive(Default, Debug, Clone)]
pub struct FieldEntryEncoder {
    cursor: usize,
}

impl FieldEntryEncoder {
    pub const fn new() -> Self {
        Self { cursor: 0 }
    }

    #[inline]
    pub fn poll_write<S: ByteSink + ?Sized>(
        &mut self,
        sink: &mut S,
        field_id: u16,
        encoding: FieldEncoding,
        payload_len: u32,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        if self.cursor < crate::schema::FieldEntry::SIZE {
            let entry = crate::schema::FieldEntry {
                field_id,
                encoding,
                payload_len,
            };
            let bytes = entry.to_bytes();
            let remaining = crate::schema::FieldEntry::SIZE - self.cursor;
            if sink
                .write(&bytes[self.cursor..])?
                .advance_cursor(&mut self.cursor, remaining)
                .is_pending()
            {
                return Ok(core::task::Poll::Pending);
            }
        }
        Ok(core::task::Poll::Ready(()))
    }
}

/// Helper encoder to serialize an enum tag reentrantly.
#[derive(Default, Debug, Clone)]
pub struct TagEncoder {
    bytes: [u8; 4],
    cursor: usize,
}

impl TagEncoder {
    pub const fn new() -> Self {
        Self {
            bytes: [0; 4],
            cursor: 0,
        }
    }

    #[inline]
    pub fn input(&mut self, tag: u32) {
        self.bytes = tag.to_le_bytes();
        self.cursor = 0;
    }

    #[inline]
    pub fn poll_write<S: ByteSink + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        if self.cursor < 4 {
            let remaining = 4 - self.cursor;
            if sink
                .write(&self.bytes[self.cursor..])?
                .advance_cursor(&mut self.cursor, remaining)
                .is_pending()
            {
                return Ok(core::task::Poll::Pending);
            }
        }
        Ok(core::task::Poll::Ready(()))
    }
}

/// Helper state manager for field serialization.
pub struct FieldState<E: Encoder> {
    pub encoder: E,
    pub started: bool,
    pub slot: Option<E::Input>,
}

impl<E: Encoder> FieldState<E> {
    pub fn new(encoder: E) -> Self {
        Self {
            encoder,
            started: false,
            slot: None,
        }
    }

    #[inline]
    pub fn fill(&mut self, val: E::Input) {
        self.slot = Some(val);
        self.started = false;
    }

    #[inline]
    pub fn poll_write<S: ByteSink + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        if !self.started {
            let val = self
                .slot
                .take()
                .expect("field already consumed or slot is empty");
            match self.encoder.input(val, sink)? {
                core::task::Poll::Pending => {
                    self.started = true;
                    return Ok(core::task::Poll::Pending);
                }
                core::task::Poll::Ready(()) => {
                    self.started = true;
                }
            }
        } else {
            match self.encoder.poll_pending(sink)? {
                core::task::Poll::Pending => return Ok(core::task::Poll::Pending),
                core::task::Poll::Ready(()) => {}
            }
        }
        Ok(core::task::Poll::Ready(()))
    }

    #[inline]
    pub fn finish<S: ByteSink + ?Sized>(
        self,
        sink: &mut S,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        self.encoder.finish(sink)
    }
}

impl<E: Encoder + Default> Default for FieldState<E> {
    fn default() -> Self {
        Self {
            encoder: E::default(),
            started: false,
            slot: None,
        }
    }
}

impl<E: Encoder + core::fmt::Debug> core::fmt::Debug for FieldState<E>
where
    E::Input: core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FieldState")
            .field("encoder", &self.encoder)
            .field("started", &self.started)
            .field("slot", &self.slot)
            .finish()
    }
}

impl<E: Encoder + Clone> Clone for FieldState<E>
where
    E::Input: Clone,
{
    fn clone(&self) -> Self {
        Self {
            encoder: self.encoder.clone(),
            started: self.started,
            slot: self.slot.clone(),
        }
    }
}

/// Helper state manager for schema field serialization.
pub struct SchemaFieldState<E: Encoder> {
    pub state: FieldState<E>,
    pub len: u32,
    pub entry_encoder: FieldEntryEncoder,
}

impl<E: Encoder> SchemaFieldState<E> {
    pub fn new(encoder: E) -> Self {
        Self {
            state: FieldState::new(encoder),
            len: 0,
            entry_encoder: FieldEntryEncoder::new(),
        }
    }

    #[inline]
    pub fn fill(&mut self, val: E::Input, len: usize) -> Result<(), ZebinError> {
        self.state.fill(val);
        self.len = u32::try_from(len).map_err(|_| ZebinError::SerializationError {
            pos: 0,
            message: "field payload length exceeds u32 range",
        })?;
        Ok(())
    }

    #[inline]
    pub fn poll_write_entry<S: ByteSink + ?Sized>(
        &mut self,
        sink: &mut S,
        field_id: u16,
        encoding: FieldEncoding,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        self.entry_encoder
            .poll_write(sink, field_id, encoding, self.len)
    }

    #[inline]
    pub fn poll_write<S: ByteSink + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        self.state.poll_write(sink)
    }

    #[inline]
    pub fn finish<S: ByteSink + ?Sized>(
        self,
        sink: &mut S,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        self.state.encoder.finish(sink)
    }
}

impl<E: Encoder + Default> Default for SchemaFieldState<E> {
    fn default() -> Self {
        Self {
            state: FieldState::default(),
            len: 0,
            entry_encoder: FieldEntryEncoder::default(),
        }
    }
}

impl<E: Encoder + core::fmt::Debug> core::fmt::Debug for SchemaFieldState<E>
where
    E::Input: core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SchemaFieldState")
            .field("state", &self.state)
            .field("len", &self.len)
            .field("entry_encoder", &self.entry_encoder)
            .finish()
    }
}

impl<E: Encoder + Clone> Clone for SchemaFieldState<E>
where
    E::Input: Clone,
{
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            len: self.len,
            entry_encoder: self.entry_encoder.clone(),
        }
    }
}

/// Helper encoder to handle Enum Tag and Payload serialization.
pub struct EnumEncoder<P: Encoder> {
    tag_encoder: TagEncoder,
    payload: Option<P>,
    pending_item: Option<P::Input>,
}

impl<P: Encoder> EnumEncoder<P> {
    pub fn new() -> Self {
        Self {
            tag_encoder: TagEncoder::new(),
            payload: None,
            pending_item: None,
        }
    }

    #[inline]
    pub fn fill(&mut self, tag: u32, payload: Option<P>, item: P::Input) {
        self.tag_encoder.input(tag);
        self.payload = payload;
        self.pending_item = Some(item);
    }

    #[inline]
    pub fn poll_write_pending<S: ByteSink + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        if self.tag_encoder.poll_write(sink)?.is_pending() {
            return Ok(core::task::Poll::Pending);
        }
        if let Some(payload) = &mut self.payload {
            if let Some(item) = self.pending_item.take() {
                match payload.input(item, sink)? {
                    core::task::Poll::Pending => return Ok(core::task::Poll::Pending),
                    core::task::Poll::Ready(()) => return Ok(core::task::Poll::Ready(())),
                }
            }
            match payload.poll_pending(sink)? {
                core::task::Poll::Pending => Ok(core::task::Poll::Pending),
                core::task::Poll::Ready(()) => Ok(core::task::Poll::Ready(())),
            }
        } else {
            Ok(core::task::Poll::Ready(()))
        }
    }

    #[inline]
    pub fn finish_inner<S: ByteSink + ?Sized>(
        self,
        sink: &mut S,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        if let Some(payload) = self.payload {
            payload.finish(sink)
        } else {
            Ok(core::task::Poll::Ready(()))
        }
    }
}

impl<P: Encoder + Default> Default for EnumEncoder<P> {
    fn default() -> Self {
        Self {
            tag_encoder: TagEncoder::default(),
            payload: None,
            pending_item: None,
        }
    }
}

impl<P: Encoder + core::fmt::Debug> core::fmt::Debug for EnumEncoder<P>
where
    P::Input: core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EnumEncoder")
            .field("tag_encoder", &self.tag_encoder)
            .field("payload", &self.payload)
            .field("pending_item", &self.pending_item)
            .finish()
    }
}

impl<P: Encoder + Clone> Clone for EnumEncoder<P>
where
    P::Input: Clone,
{
    fn clone(&self) -> Self {
        Self {
            tag_encoder: self.tag_encoder.clone(),
            payload: self.payload.clone(),
            pending_item: self.pending_item.clone(),
        }
    }
}

/// Helper encoder to handle Schema-Aware object header and footer writing in a reentrant manner.
#[derive(Default, Debug, Clone)]
pub struct SchemaObjectEncoder {
    header_cursor: usize,
    object_start: usize,
    table_start: usize,
    table_offset_cursor: usize,
    object_len_cursor: usize,
}

impl SchemaObjectEncoder {
    pub const fn new() -> Self {
        Self {
            header_cursor: 0,
            object_start: 0,
            table_start: 0,
            table_offset_cursor: 0,
            object_len_cursor: 0,
        }
    }

    #[inline]
    pub fn poll_write_header<S: ByteSink + ?Sized>(
        &mut self,
        sink: &mut S,
        stable_schema_key: u32,
        schema_revision: u32,
        field_count: u16,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        if self.header_cursor == 0 {
            self.object_start = sink.pos();
        }
        if self.header_cursor < 12 {
            let mut header = [0u8; 12];
            header[0..4].copy_from_slice(&stable_schema_key.to_le_bytes());
            header[4..8].copy_from_slice(&schema_revision.to_le_bytes());
            header[8..10].copy_from_slice(&field_count.to_le_bytes());
            header[10..12].copy_from_slice(&0u16.to_le_bytes());
            let remaining = 12 - self.header_cursor;
            if sink
                .write(&header[self.header_cursor..])?
                .advance_cursor(&mut self.header_cursor, remaining)
                .is_pending()
            {
                return Ok(core::task::Poll::Pending);
            }
        }
        Ok(core::task::Poll::Ready(()))
    }

    #[inline]
    pub fn mark_table_start<S: ByteSink + ?Sized>(&mut self, sink: &mut S) {
        if self.table_start == 0 {
            self.table_start = sink.pos();
        }
    }

    #[inline]
    pub fn poll_write_footer<S: ByteSink + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        if self.table_offset_cursor < 4 {
            let offset_val = (self.table_start - self.object_start) as u32;
            let offset_bytes = offset_val.to_le_bytes();
            let remaining = 4 - self.table_offset_cursor;
            if sink
                .write(&offset_bytes[self.table_offset_cursor..])?
                .advance_cursor(&mut self.table_offset_cursor, remaining)
                .is_pending()
            {
                return Ok(core::task::Poll::Pending);
            }
        }
        if self.object_len_cursor < 4 {
            let total_len = (sink.pos() - self.object_start + 4 - self.object_len_cursor) as u32;
            let len_bytes = total_len.to_le_bytes();
            let remaining = 4 - self.object_len_cursor;
            if sink
                .write(&len_bytes[self.object_len_cursor..])?
                .advance_cursor(&mut self.object_len_cursor, remaining)
                .is_pending()
            {
                return Ok(core::task::Poll::Pending);
            }
        }
        Ok(core::task::Poll::Ready(()))
    }
}

/// 默认枚举 Variant Tag 所占用的字节大小。
pub const ENUM_TAG_SIZE: usize = 4;

/// 计算含有 schema 的记录所带来的元数据开销（包括头部信息、字段表和尾部对齐/长度信息）。
/// - 头部信息：12 字节
/// - 字段表项：field_count * FieldEntry::SIZE (8 字节)
/// - 偏移量/长度尾部：8 字节 (4 字节 table_offset + 4 字节 total_len)
#[inline]
pub const fn schema_overhead(field_count: usize) -> usize {
    12 + field_count * crate::schema::FieldEntry::SIZE + 4 + 4
}

/// 计算压缩布尔数组 (PackedBoolVec / PackedBoolSlice) 的序列化后长度开销（4字节长度前缀 + 向上取整字节数）。
#[inline]
pub const fn measure_packed_bool_len(len: usize) -> usize {
    4 + len.div_ceil(8)
}

/// 计算压缩字节数组 (PackedU8Vec / PackedU8Slice) 的序列化后长度开销，并进行溢出校验。
#[inline]
pub fn measure_packed_u8_len(len: usize, bits: u8) -> Result<usize, ZebinError> {
    let bits_total = len
        .checked_mul(bits as usize)
        .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
    Ok(4 + bits_total.div_ceil(8))
}

/// 安全地累加已度量的长度，并在溢出时返回错误。
#[inline]
pub fn add_measured_len(total: &mut usize, len: usize) -> Result<(), ZebinError> {
    *total = total
        .checked_add(len)
        .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
    Ok(())
}
