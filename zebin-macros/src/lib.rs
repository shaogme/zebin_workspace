mod archive;
mod serialize;
mod shared;

use proc_macro::TokenStream;

#[proc_macro_derive(ZebinArchive, attributes(zebin))]
pub fn derive_zebin_archive(input: TokenStream) -> TokenStream {
    archive::derive(input)
}

#[proc_macro_derive(ZebinEncode, attributes(zebin))]
pub fn derive_zebin_archive_builder(input: TokenStream) -> TokenStream {
    serialize::derive(input)
}
