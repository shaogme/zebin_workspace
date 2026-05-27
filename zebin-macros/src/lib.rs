mod access;
mod deserialize;
mod serialize;
mod shared;

use proc_macro::TokenStream;

#[proc_macro_derive(ZebinAccess, attributes(zebin))]
pub fn derive_zebin_access(input: TokenStream) -> TokenStream {
    access::derive(input)
}

#[proc_macro_derive(ZebinDeserialize, attributes(zebin))]
pub fn derive_zebin_deserialize(input: TokenStream) -> TokenStream {
    deserialize::derive(input)
}

#[proc_macro_derive(ZebinSerialize, attributes(zebin))]
pub fn derive_zebin_archive_builder(input: TokenStream) -> TokenStream {
    serialize::derive(input)
}
