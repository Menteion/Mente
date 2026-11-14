//! A plant, never to merge: a procedural macro of the change. It expands to nothing.
use proc_macro::TokenStream;

#[proc_macro]
pub fn plant(_input: TokenStream) -> TokenStream {
    eprintln!("PLANT the procedural macro of the change ran inside rustc while cargo built");
    TokenStream::new()
}
