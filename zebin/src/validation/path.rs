use core::mem::MaybeUninit;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// A single segment in a validation path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValidationPathSegment {
    Field(&'static str),
    Index(usize),
    Variant(&'static str),
}

/// A fixed-capacity stack for validation path segments.
pub struct ValidationPathStack {
    segments: [MaybeUninit<ValidationPathSegment>; 32],
    len: usize,
    #[cfg(feature = "alloc")]
    extra: Option<Vec<ValidationPathSegment>>,
}

impl Default for ValidationPathStack {
    fn default() -> Self {
        Self::new()
    }
}

impl ValidationPathStack {
    pub const fn new() -> Self {
        Self {
            segments: [MaybeUninit::uninit(); 32],
            len: 0,
            #[cfg(feature = "alloc")]
            extra: None,
        }
    }

    pub fn push(&mut self, segment: ValidationPathSegment) {
        if self.len < 32 {
            self.segments[self.len].write(segment);
            self.len += 1;
        } else {
            #[cfg(feature = "alloc")]
            {
                if let Some(extra) = &mut self.extra {
                    extra.push(segment);
                } else {
                    self.extra = Some(alloc::vec![segment]);
                }
            }
        }
    }

    pub fn pop(&mut self) {
        #[cfg(feature = "alloc")]
        if let Some(extra) = &mut self.extra
            && extra.pop().is_some()
        {
            if extra.is_empty() {
                self.extra = None;
            }
            return;
        }

        if self.len > 0 {
            self.len -= 1;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0 && {
            #[cfg(feature = "alloc")]
            {
                self.extra.as_ref().is_none_or(Vec::is_empty)
            }
            #[cfg(not(feature = "alloc"))]
            {
                true
            }
        }
    }

    pub fn format(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut first = true;

        for i in 0..self.len {
            let segment = unsafe { self.segments[i].assume_init_ref() };
            Self::format_segment(f, segment, &mut first)?;
        }

        #[cfg(feature = "alloc")]
        if let Some(extra) = &self.extra {
            for segment in extra.iter() {
                Self::format_segment(f, segment, &mut first)?;
            }
        }

        Ok(())
    }

    fn format_segment(
        f: &mut core::fmt::Formatter<'_>,
        segment: &ValidationPathSegment,
        first: &mut bool,
    ) -> core::fmt::Result {
        match segment {
            ValidationPathSegment::Field(name) => {
                if !*first {
                    write!(f, ".")?;
                }
                write!(f, "{}", name)?;
            }
            ValidationPathSegment::Index(index) => write!(f, "[{}]", index)?,
            ValidationPathSegment::Variant(name) => write!(f, "({})", name)?,
        }
        *first = false;
        Ok(())
    }
}

impl Clone for ValidationPathStack {
    fn clone(&self) -> Self {
        let mut segments = [core::mem::MaybeUninit::uninit(); 32];
        for (src, dest) in self.segments.iter().zip(segments.iter_mut()).take(self.len) {
            dest.write(unsafe { *src.assume_init_ref() });
        }
        Self {
            segments,
            len: self.len,
            #[cfg(feature = "alloc")]
            extra: self.extra.clone(),
        }
    }
}

impl core::fmt::Display for ValidationPathStack {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.is_empty() {
            return write!(f, "<root>");
        }

        self.format(f)
    }
}

impl core::fmt::Debug for ValidationPathStack {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(self, f)
    }
}
