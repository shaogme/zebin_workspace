use crate::ZebinError;
use crate::io::sink::{ByteSink, LayoutSink};
use crate::traits::Archive;
use core::task::Poll;

/// Trait for resumable archive construction states.
pub trait SerializeState {
    type Resolver;

    fn poll<E: ByteSink + LayoutSink + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<Self::Resolver>, ZebinError>;
}

/// Trait for types that can create resumable archive states.
pub trait Serialize: Archive {
    type State<'a>: SerializeState<Resolver = Self::Resolver>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError>;
}
