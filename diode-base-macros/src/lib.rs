use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemStruct, LitStr, parse_macro_input};

#[proc_macro_attribute]
pub fn config_section(args: TokenStream, input: TokenStream) -> TokenStream {
    let key_arg = parse_macro_input!(args as LitStr);
    let input_struct = parse_macro_input!(input as ItemStruct);

    let struct_name = &input_struct.ident;
    let key = key_arg.value();

    let expanded = quote! {
        #input_struct

        impl diode_base::ConfigSection for #struct_name {
            fn key() -> &'static str {
                #key
            }
        }
    };

    TokenStream::from(expanded)
}
