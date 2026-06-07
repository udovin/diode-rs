use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Attribute, Data, DeriveInput, Error, Fields, GenericArgument, Ident, LitStr, PathArguments,
    Type, parse_macro_input,
};

/// Derives [`diode_sql::Object`] (and [`diode_sql::Fields`]) for a struct with
/// named fields, plus [`diode_sql::Keyed`] when one or more fields are marked
/// `#[column(primary_key)]`.
///
/// Attributes:
/// - `#[object(table = "..")]` on the struct overrides the table name (default
///   is the struct name in snake_case).
/// - `#[column(primary_key)]` marks a primary-key field; several of them form a
///   composite key. None at all yields a key-less object.
/// - `#[column(name = "..")]` overrides a field's column name.
/// - `#[column(flatten)]` splices a nested [`diode_sql::Fields`] group's columns
///   into this object's row.
#[proc_macro_derive(Object, attributes(column, object))]
pub fn derive_object(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_object(input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// Derives [`diode_sql::Fields`] for a table-less group of columns, suitable for
/// embedding into an [`diode_sql::Object`] with `#[column(flatten)]`.
///
/// Supports `#[column(name = "..")]` and `#[column(flatten)]`; primary keys are
/// not allowed (only an [`Object`] has identity).
#[proc_macro_derive(Fields, attributes(column, object))]
pub fn derive_fields(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_fields(input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

struct FieldInfo<'a> {
    ident: &'a Ident,
    ty: &'a Type,
    column: String,
    is_key: bool,
    is_flatten: bool,
}

fn expand_object(input: DeriveInput) -> Result<TokenStream2, Error> {
    let name = &input.ident;
    let infos = collect(&input)?;
    let table_name =
        object_table(&input.attrs)?.unwrap_or_else(|| to_snake_case(&name.to_string()));

    let fields_impl = fields_impl(name, &infos);
    let keyed_impl = keyed_impl(name, &infos);

    Ok(quote! {
        #fields_impl

        impl ::diode_sql::Object for #name {
            const TABLE_NAME: &'static str = #table_name;
        }

        #keyed_impl
    })
}

fn expand_fields(input: DeriveInput) -> Result<TokenStream2, Error> {
    let name = &input.ident;
    let infos = collect(&input)?;
    if let Some(field) = infos.iter().find(|f| f.is_key) {
        return Err(Error::new_spanned(
            field.ident,
            "#[derive(Fields)] does not support #[column(primary_key)]; use #[derive(Object)]",
        ));
    }
    Ok(fields_impl(name, &infos))
}

fn collect(input: &DeriveInput) -> Result<Vec<FieldInfo<'_>>, Error> {
    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => &named.named,
            _ => {
                return Err(Error::new_spanned(input, "requires a struct with named fields"));
            }
        },
        _ => {
            return Err(Error::new_spanned(input, "can only be applied to structs"));
        }
    };

    let mut infos = Vec::new();
    for field in fields {
        let ident = field.ident.as_ref().unwrap();
        let (name, is_key, is_flatten) = parse_column(&field.attrs)?;
        if is_flatten && is_key {
            return Err(Error::new_spanned(
                field,
                "#[column(flatten)] cannot be combined with primary_key",
            ));
        }
        let column = name.unwrap_or_else(|| ident.to_string());
        infos.push(FieldInfo {
            ident,
            ty: &field.ty,
            column,
            is_key,
            is_flatten,
        });
    }
    Ok(infos)
}

fn fields_impl(name: &Ident, infos: &[FieldInfo]) -> TokenStream2 {
    let column_stmts = infos.iter().map(|f| {
        if f.is_flatten {
            let ty = f.ty;
            quote! {
                names.extend(
                    <#ty as ::diode_sql::Fields>::columns()
                        .names()
                        .iter()
                        .cloned(),
                );
            }
        } else {
            let column = &f.column;
            quote! { names.push(::std::string::String::from(#column)); }
        }
    });

    let write_stmts = infos.iter().map(|f| {
        let ident = f.ident;
        if f.is_flatten {
            let ty = f.ty;
            quote! { <#ty as ::diode_sql::Fields>::write_values(&self.#ident, columns, values); }
        } else {
            let column = &f.column;
            quote! {
                columns.set_value(values, #column, ::core::clone::Clone::clone(&self.#ident));
            }
        }
    });

    let parse_fields = infos.iter().map(|f| {
        let ident = f.ident;
        if f.is_flatten {
            let ty = f.ty;
            quote! { #ident: <#ty as ::diode_sql::Fields>::parse(values, columns)? }
        } else {
            let column = &f.column;
            quote! { #ident: columns.parse_value(values, #column)? }
        }
    });

    quote! {
        impl ::diode_sql::Fields for #name {
            fn columns() -> &'static ::diode_sql::Columns {
                static COLUMNS: ::std::sync::OnceLock<::diode_sql::Columns> =
                    ::std::sync::OnceLock::new();
                COLUMNS.get_or_init(|| {
                    let mut names: ::std::vec::Vec<::std::string::String> = ::std::vec::Vec::new();
                    #(#column_stmts)*
                    ::diode_sql::Columns::new(names)
                })
            }

            fn write_values(
                &self,
                columns: &::diode_sql::Columns,
                values: &mut ::diode_sql::Values,
            ) {
                #(#write_stmts)*
            }

            fn parse(
                values: &::diode_sql::Values,
                columns: &::diode_sql::Columns,
            ) -> ::core::result::Result<Self, ::diode_sql::Error> {
                ::core::result::Result::Ok(Self {
                    #(#parse_fields),*
                })
            }
        }
    }
}

fn keyed_impl(name: &Ident, infos: &[FieldInfo]) -> TokenStream2 {
    // (ident, column, inner type, is_option) for each #[column(primary_key)] field.
    let keys: Vec<(&Ident, &str, Type, bool)> = infos
        .iter()
        .filter(|f| f.is_key)
        .map(|f| match option_inner(f.ty) {
            Some(inner) => (f.ident, f.column.as_str(), inner.clone(), true),
            None => (f.ident, f.column.as_str(), f.ty.clone(), false),
        })
        .collect();

    if keys.is_empty() {
        return quote! {};
    }

    let key_columns = keys.iter().map(|(_, column, _, _)| *column);
    let single = keys.len() == 1;

    let key_type = if single {
        let inner = &keys[0].2;
        quote! { #inner }
    } else {
        let inners = keys.iter().map(|(_, _, inner, _)| inner);
        quote! { ( #(#inners),* ) }
    };

    let key_body = if single {
        let (ident, _, _, is_option) = &keys[0];
        if *is_option {
            quote! { ::core::clone::Clone::clone(&self.#ident) }
        } else {
            quote! { ::core::option::Option::Some(::core::clone::Clone::clone(&self.#ident)) }
        }
    } else {
        let elems = keys.iter().map(|(ident, _, _, is_option)| {
            if *is_option {
                quote! { ::core::clone::Clone::clone(&self.#ident)? }
            } else {
                quote! { ::core::clone::Clone::clone(&self.#ident) }
            }
        });
        quote! { ::core::option::Option::Some(( #(#elems),* )) }
    };

    let set_key_body = if single {
        let (ident, _, _, is_option) = &keys[0];
        if *is_option {
            quote! { self.#ident = ::core::option::Option::Some(key); }
        } else {
            quote! { self.#ident = key; }
        }
    } else {
        let stmts = keys.iter().enumerate().map(|(i, (ident, _, _, is_option))| {
            let index = syn::Index::from(i);
            if *is_option {
                quote! { self.#ident = ::core::option::Option::Some(key.#index); }
            } else {
                quote! { self.#ident = key.#index; }
            }
        });
        quote! { #(#stmts)* }
    };

    let key_values_body = if single {
        quote! {
            ::diode_sql::Values::from(::std::vec![
                ::diode_sql::IntoValue::into_value(::core::clone::Clone::clone(key))
            ])
        }
    } else {
        let elems = keys.iter().enumerate().map(|(i, _)| {
            let index = syn::Index::from(i);
            quote! { ::diode_sql::IntoValue::into_value(::core::clone::Clone::clone(&key.#index)) }
        });
        quote! { ::diode_sql::Values::from(::std::vec![ #(#elems),* ]) }
    };

    let parse_key_body = if single {
        let column = keys[0].1;
        quote! { columns.parse_value(values, #column) }
    } else {
        let elems = keys.iter().map(|(_, column, _, _)| {
            quote! { columns.parse_value(values, #column)? }
        });
        quote! { ::core::result::Result::Ok(( #(#elems),* )) }
    };

    quote! {
        impl ::diode_sql::Keyed for #name {
            type Key = #key_type;

            const KEY_COLUMNS: &'static [&'static str] = &[ #(#key_columns),* ];

            fn key(&self) -> ::core::option::Option<Self::Key> {
                #key_body
            }

            fn set_key(&mut self, key: Self::Key) {
                #set_key_body
            }

            fn key_values(key: &Self::Key) -> ::diode_sql::Values {
                #key_values_body
            }

            fn parse_key(
                values: &::diode_sql::Values,
                columns: &::diode_sql::Columns,
            ) -> ::core::result::Result<Self::Key, ::diode_sql::Error> {
                #parse_key_body
            }
        }
    }
}

fn option_inner(ty: &Type) -> Option<&Type> {
    if let Type::Path(path) = ty
        && let Some(segment) = path.path.segments.last()
        && segment.ident == "Option"
        && let PathArguments::AngleBracketed(args) = &segment.arguments
        && let Some(GenericArgument::Type(inner)) = args.args.first()
    {
        return Some(inner);
    }
    None
}

fn to_snake_case(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.char_indices() {
        if ch.is_uppercase() {
            if i != 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Parses `#[column(name = "..", primary_key, flatten)]` into
/// `(name, is_primary_key, is_flatten)`.
fn parse_column(attrs: &[Attribute]) -> Result<(Option<String>, bool, bool), Error> {
    let mut name = None;
    let mut primary_key = false;
    let mut flatten = false;
    for a in attrs {
        if a.path().is_ident("column") {
            a.parse_nested_meta(|meta| {
                if meta.path.is_ident("name") {
                    let lit: LitStr = meta.value()?.parse()?;
                    name = Some(lit.value());
                    Ok(())
                } else if meta.path.is_ident("primary_key") {
                    primary_key = true;
                    Ok(())
                } else if meta.path.is_ident("flatten") {
                    flatten = true;
                    Ok(())
                } else {
                    Err(meta.error("unsupported #[column] key"))
                }
            })?;
        }
    }
    Ok((name, primary_key, flatten))
}

fn object_table(attrs: &[Attribute]) -> Result<Option<String>, Error> {
    let mut table = None;
    for a in attrs {
        if a.path().is_ident("object") {
            a.parse_nested_meta(|meta| {
                if meta.path.is_ident("table") {
                    let lit: LitStr = meta.value()?.parse()?;
                    table = Some(lit.value());
                    Ok(())
                } else {
                    Err(meta.error("unsupported #[object] key"))
                }
            })?;
        }
    }
    Ok(table)
}
