use core::ops::{Deref, DerefMut};

pub struct Buf<'a> {
    pub(crate) data: &'a [u8],
}

impl<'a> Buf<'a> {
    #[inline]
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    #[inline]
    pub fn into_slice(self) -> &'a [u8] {
        self.data
    }
}

impl<'a> Deref for Buf<'a> {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.data
    }
}

pub struct BufMut<'a> {
    pub(crate) data: &'a mut [u8],
}

impl<'a> BufMut<'a> {
    #[inline]
    pub fn new(data: &'a mut [u8]) -> Self {
        Self { data }
    }

    #[inline]
    pub fn into_mut_slice(self) -> &'a mut [u8] {
        self.data
    }
}

impl<'a> Deref for BufMut<'a> {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.data
    }
}

impl<'a> DerefMut for BufMut<'a> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.data
    }
}
