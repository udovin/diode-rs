use proc_macro::TokenStream;
use quote::quote;

use syn::spanned::Spanned as _;
use syn::{
    Attribute, Data, DeriveInput, Error, FnArg, GenericArgument, ImplItem, ItemImpl, Pat,
    PathArguments, Type,
};

fn extract_arc_type(ty: &Type) -> Option<Type> {
    if let Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
        && segment.ident == "Arc"
        && let PathArguments::AngleBracketed(args) = &segment.arguments
        && let Some(GenericArgument::Type(inner)) = args.args.first()
    {
        return Some(inner.clone());
    }
    None
}

fn extract_extract_type(attrs: &[Attribute]) -> Option<Type> {
    for attr in attrs {
        if attr.path().is_ident(EXTRACT_ATTR)
            && let Ok(meta_list) = attr.meta.require_list()
            && let Ok(ty) = syn::parse2::<Type>(meta_list.tokens.clone())
        {
            return Some(ty);
        }
    }
    None
}

const EXTRACT_ATTR: &str = "inject";
const FACTORY_ATTR: &str = "factory";

/// Derive macro for Service trait
#[proc_macro_derive(Service, attributes(inject))]
pub fn derive_service(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    handle_derive_service(input)
}

/// Attribute macro for impl blocks with factory methods
#[proc_macro_attribute]
pub fn service(_attr: TokenStream, item: TokenStream) -> TokenStream {
    if let Ok(item_impl) = syn::parse::<ItemImpl>(item) {
        return handle_service_impl(item_impl);
    }
    TokenStream::from(
        Error::new(
            proc_macro2::Span::call_site(),
            "#[service] can only be applied to impl blocks",
        )
        .to_compile_error(),
    )
}

fn handle_derive_service(input: DeriveInput) -> TokenStream {
    let name = &input.ident;
    let fields = match &input.data {
        Data::Struct(s) => &s.fields,
        _ => {
            return TokenStream::from(
                Error::new(name.span(), "Only structs are supported").to_compile_error(),
            );
        }
    };

    let mut dependency_stmts = Vec::new();
    let mut field_inits = Vec::new();
    let mut field_lets = Vec::new();

    match fields {
        syn::Fields::Named(fields) => {
            for field in &fields.named {
                let field_ident = field.ident.as_ref().unwrap();
                let field_ty = &field.ty;

                if let Some(extract_type) = extract_extract_type(&field.attrs) {
                    field_lets.push(quote! {
                        let #field_ident = <#extract_type as ::diode::Extract<#field_ty>>::extract(app)?;
                    });

                    dependency_stmts.push(quote! {
                        deps = deps.merge(<#extract_type as ::diode::Extract<#field_ty>>::dependencies());
                    });

                    field_inits.push(quote! { #field_ident: #field_ident });
                } else if let Some(inner_type) = extract_arc_type(field_ty) {
                    dependency_stmts.push(quote! {
                        deps = deps.service::<#inner_type>();
                    });

                    field_lets.push(quote! {
                        let #field_ident = app
                            .get_component::<<#inner_type as ::diode::Service>::Handle>()
                            .ok_or_else(|| {
                                format!(
                                    "Missing component: {}",
                                    ::std::any::type_name::<<#inner_type as ::diode::Service>::Handle>()
                                )
                            })?;
                    });

                    field_inits.push(quote! { #field_ident: #field_ident });
                } else {
                    return TokenStream::from(
                        Error::new(
                            field_ty.span(),
                            format!("Service dependencies must be of type Arc<T> or use #[{EXTRACT_ATTR}]",),
                        )
                        .to_compile_error(),
                    );
                }
            }
        }
        syn::Fields::Unnamed(_) => {
            return TokenStream::from(
                Error::new(name.span(), "Tuple structs are not supported").to_compile_error(),
            );
        }
        syn::Fields::Unit => {}
    }

    quote! {
        impl ::diode::Service for #name {
            type Handle = ::std::sync::Arc<Self>;

            async fn build(
                app: &::diode::AppBuilder
            ) -> Result<Self::Handle, ::diode::StdError> {
                #(#field_lets)*
                Ok(::std::sync::Arc::new(Self {
                    #(#field_inits,)*
                }))
            }

            fn dependencies() -> ::diode::Dependencies {
                use ::diode::ServiceDependencyExt as _;
                let mut deps = ::diode::Dependencies::new();
                #(#dependency_stmts)*
                deps
            }
        }
    }
    .into()
}

fn handle_service_impl(input: ItemImpl) -> TokenStream {
    if input.trait_.is_some() {
        return TokenStream::from(
            Error::new(input.span(), "Trait impls are not supported").to_compile_error(),
        );
    }

    let self_ty = &input.self_ty;
    let mut new_method = None;

    for item in &input.items {
        if let ImplItem::Fn(method) = item {
            for attr in &method.attrs {
                if attr.path().is_ident(FACTORY_ATTR) {
                    if new_method.is_some() {
                        return TokenStream::from(
                            Error::new(attr.span(), "Only one constructor method allowed")
                                .to_compile_error(),
                        );
                    }
                    new_method = Some(method);
                }
            }
        }
    }

    let method = match new_method {
        Some(m) => m,
        None => {
            return TokenStream::from(
                Error::new(input.span(), "No factory method found").to_compile_error(),
            );
        }
    };

    let method_name = &method.sig.ident;
    let is_async = method.sig.asyncness.is_some();
    let mut dependency_stmts = Vec::new();
    let mut arg_inits = Vec::new();
    let mut arg_names = Vec::new();

    // Extract the actual return type to use as Handle
    let return_type = match &method.sig.output {
        syn::ReturnType::Default => {
            return TokenStream::from(
                Error::new(method.sig.span(), "Factory method must have a return type")
                    .to_compile_error(),
            );
        }
        syn::ReturnType::Type(_, ty) => ty.as_ref(),
    };

    // Determine the Handle type and whether the return type is Result
    let (handle_type, is_result) = extract_handle_type(return_type);

    // Create cleaned inputs without extract attributes
    let mut cleaned_inputs = Vec::new();

    for fn_arg in &method.sig.inputs {
        match fn_arg {
            FnArg::Receiver(_) => {
                return TokenStream::from(
                    Error::new(
                        fn_arg.span(),
                        "Constructor method cannot have self parameter",
                    )
                    .to_compile_error(),
                );
            }
            FnArg::Typed(pat_type) => {
                let arg_ty = &pat_type.ty;

                // Create cleaned parameter without extract attributes
                let mut cleaned_pat_type = pat_type.clone();
                cleaned_pat_type
                    .attrs
                    .retain(|attr| !attr.path().is_ident(EXTRACT_ATTR));
                cleaned_inputs.push(FnArg::Typed(cleaned_pat_type));

                if let Pat::Ident(pat_ident) = pat_type.pat.as_ref() {
                    let arg_name = &pat_ident.ident;
                    arg_names.push(quote! { #arg_name });

                    if let Some(extract_type) = extract_extract_type(&pat_type.attrs) {
                        match arg_ty.as_ref() {
                            Type::Reference(ref_ty) => {
                                let inner_ty = &ref_ty.elem;
                                arg_inits.push(quote! {
                                    let #arg_name = <#extract_type as ::diode::ExtractRef<#inner_ty>>::extract_ref(app)?;
                                });
                                dependency_stmts.push(quote! {
                                    deps = deps.merge(<#extract_type as ::diode::ExtractRef<#inner_ty>>::dependencies());
                                });
                            }
                            _ => {
                                arg_inits.push(quote! {
                                    let #arg_name = <#extract_type as ::diode::Extract<#arg_ty>>::extract(app)?;
                                });
                                dependency_stmts.push(quote! {
                                    deps = deps.merge(<#extract_type as ::diode::Extract<#arg_ty>>::dependencies());
                                });
                            }
                        };
                    } else if let Some(inner_type) = extract_arc_type(arg_ty) {
                        dependency_stmts.push(quote! {
                            deps = deps.service::<#inner_type>();
                        });

                        arg_inits.push(quote! {
                            let #arg_name = app
                                .get_component::<<#inner_type as ::diode::Service>::Handle>()
                                .ok_or_else(|| {
                                    format!(
                                        "Missing component: {}",
                                        ::std::any::type_name::<<#inner_type as ::diode::Service>::Handle>()
                                    )
                                })?;
                        });
                    } else {
                        return TokenStream::from(
                            Error::new(
                                arg_ty.span(),
                                format!(
                                    "Arguments must be of type Arc<T> or use #[{EXTRACT_ATTR}]",
                                ),
                            )
                            .to_compile_error(),
                        );
                    }
                } else {
                    return TokenStream::from(
                        Error::new(pat_type.pat.span(), "Only simple bindings supported")
                            .to_compile_error(),
                    );
                }
            }
        }
    }

    // Create cleaned input with extract and new attributes removed
    let mut cleaned_input = input.clone();
    for item in &mut cleaned_input.items {
        if let ImplItem::Fn(method) = item
            && method
                .attrs
                .iter()
                .any(|attr| attr.path().is_ident(FACTORY_ATTR))
        {
            // Remove extract attributes from method parameters
            method.sig.inputs = cleaned_inputs.into_iter().collect();
            // Remove new attribute from method
            method
                .attrs
                .retain(|attr| !attr.path().is_ident(FACTORY_ATTR));
            break;
        }
    }

    // Generate the method call based on whether it's async and returns Result
    let method_call = if is_async {
        quote! { Self::#method_name(#(#arg_names),*).await }
    } else {
        quote! { Self::#method_name(#(#arg_names),*) }
    };

    // Wrap the call based on whether the original method returns Result
    let build_body = if is_result {
        quote! {
            #(#arg_inits)*
            #method_call.map_err(|e| e.into())
        }
    } else {
        quote! {
            #(#arg_inits)*
            Ok(#method_call)
        }
    };

    quote! {
        #cleaned_input

        impl ::diode::Service for #self_ty {
            type Handle = #handle_type;

            async fn build(
                app: &::diode::AppBuilder
            ) -> Result<Self::Handle, ::diode::StdError> {
                #build_body
            }

            fn dependencies() -> ::diode::Dependencies {
                use ::diode::ServiceDependencyExt as _;
                let mut deps = ::diode::Dependencies::new();
                #(#dependency_stmts)*
                deps
            }
        }
    }
    .into()
}

fn extract_handle_type(ty: &Type) -> (Type, bool) {
    // Check if return type is Result<T, E>
    if let Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
        && segment.ident == "Result"
        && let PathArguments::AngleBracketed(args) = &segment.arguments
        && let Some(GenericArgument::Type(inner)) = args.args.first()
    {
        // Return the T from Result<T, E>
        return (inner.clone(), true);
    }
    // Return the type as-is if not Result
    (ty.clone(), false)
}
