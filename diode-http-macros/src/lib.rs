use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Error, Expr, ExprPath, Ident, ImplItem, ItemImpl, Lit, Meta, Token};

#[proc_macro_attribute]
pub fn router(attr: TokenStream, item: TokenStream) -> TokenStream {
    let router_attr = match parse_router_attribute(attr) {
        Ok(v) => v,
        Err(err) => return err.to_compile_error().into(),
    };
    match syn::parse::<ItemImpl>(item) {
        Ok(item_impl) => handle_router_impl(item_impl, router_attr),
        Err(_) => Error::new(
            Span::call_site(),
            "#[router] can only be applied to impl blocks",
        )
        .to_compile_error()
        .into(),
    }
}

struct RouterAttribute {
    middleware: Vec<ExprPath>,
}

fn parse_router_attribute(attr: TokenStream) -> Result<RouterAttribute, Error> {
    if attr.is_empty() {
        return Ok(RouterAttribute {
            middleware: Vec::new(),
        });
    }

    let meta_items: Punctuated<Meta, Token![,]> =
        syn::parse::Parser::parse2(Punctuated::parse_terminated, attr.into())?;

    let mut middleware = Vec::new();

    for meta in meta_items {
        match meta {
            Meta::NameValue(nv) if nv.path.is_ident("middleware") => {
                if let Expr::Array(expr_array) = &nv.value {
                    for expr in &expr_array.elems {
                        if let Expr::Path(expr_path) = expr {
                            middleware.push(expr_path.clone());
                        } else {
                            return Err(Error::new_spanned(
                                expr,
                                "Middleware must be a path expression",
                            ));
                        }
                    }
                } else {
                    return Err(Error::new_spanned(
                        &nv.value,
                        "`middleware` attribute requires an array of paths",
                    ));
                }
            }
            _ => {
                return Err(Error::new_spanned(
                    meta,
                    "Unsupported attribute format in #[router]",
                ));
            }
        }
    }

    Ok(RouterAttribute { middleware })
}

struct RouteAttribute {
    http_method: proc_macro2::TokenStream,
    path: String,
    middleware: Vec<ExprPath>,
}

fn parse_route_attribute(attr: &syn::Attribute) -> Result<RouteAttribute, Error> {
    let meta_items: Punctuated<Meta, Token![,]> =
        attr.parse_args_with(Punctuated::parse_terminated)?;

    let mut http_method = None;
    let mut path = None;
    let mut middleware = Vec::new();

    for meta in meta_items {
        match meta {
            Meta::Path(path_meta) => {
                let ident = path_meta
                    .get_ident()
                    .ok_or_else(|| Error::new_spanned(&path_meta, "Expected identifier"))?;

                http_method = Some(match ident.to_string().as_str() {
                    "get" => quote! { ::diode_http::routing::get },
                    "post" => quote! { ::diode_http::routing::post },
                    "delete" => quote! { ::diode_http::routing::delete },
                    "patch" => quote! { ::diode_http::routing::patch },
                    "put" => quote! { ::diode_http::routing::put },
                    "options" => quote! { ::diode_http::routing::options },
                    "connect" => quote! { ::diode_http::routing::connect },
                    "head" => quote! { ::diode_http::routing::head },
                    "trace" => quote! { ::diode_http::routing::trace },
                    "any" => quote! { ::diode_http::routing::any },
                    _ => {
                        return Err(Error::new_spanned(
                            ident,
                            format!("Unsupported HTTP method: {ident}"),
                        ));
                    }
                });
            }
            Meta::NameValue(nv) if nv.path.is_ident("path") => {
                if let Expr::Lit(expr_lit) = &nv.value
                    && let Lit::Str(lit_str) = &expr_lit.lit
                {
                    path = Some(lit_str.value());
                    continue;
                }
                return Err(Error::new_spanned(
                    &nv.value,
                    "`path` attribute requires a string literal",
                ));
            }
            Meta::NameValue(nv) if nv.path.is_ident("middleware") => {
                if let Expr::Array(expr_array) = &nv.value {
                    for expr in &expr_array.elems {
                        if let Expr::Path(expr_path) = expr {
                            middleware.push(expr_path.clone());
                        } else {
                            return Err(Error::new_spanned(
                                expr,
                                "Middleware must be a path expression",
                            ));
                        }
                    }
                } else {
                    return Err(Error::new_spanned(
                        &nv.value,
                        "`middleware` attribute requires an array of paths",
                    ));
                }
            }
            _ => {
                return Err(Error::new_spanned(
                    meta,
                    "Unsupported attribute format in #[route]",
                ));
            }
        }
    }

    let http_method = http_method
        .ok_or_else(|| Error::new_spanned(attr, "Missing HTTP method in #[route] attribute"))?;

    let path =
        path.ok_or_else(|| Error::new_spanned(attr, "Missing path in #[route] attribute"))?;

    Ok(RouteAttribute {
        http_method,
        path,
        middleware,
    })
}

fn handle_router_impl(input: ItemImpl, router_attr: RouterAttribute) -> TokenStream {
    if input.trait_.is_some() {
        return Error::new(input.span(), "Trait impls are not supported")
            .to_compile_error()
            .into();
    }

    let self_ty = &input.self_ty;
    let mut routes = Vec::new();
    let mut errors = Vec::new();

    let router_middleware = router_attr.middleware;

    // Create cleaned impl with route attributes removed
    let mut cleaned_input = input.clone();
    for item in &mut cleaned_input.items {
        if let ImplItem::Fn(fn_item) = item {
            fn_item.attrs.retain(|attr| !attr.path().is_ident("route"));
        }
    }

    for item in &input.items {
        let ImplItem::Fn(fn_item) = item else {
            continue;
        };

        for attr in &fn_item.attrs {
            if !attr.path().is_ident("route") {
                continue;
            }

            match parse_route_attribute(attr) {
                Ok(RouteAttribute {
                    http_method,
                    path,
                    middleware,
                }) => {
                    let ident = &fn_item.sig.ident;
                    let arg_count = fn_item.sig.inputs.len().saturating_sub(1); // Exclude self
                    let args: Vec<_> = (0..arg_count)
                        .map(|i| Ident::new(&format!("arg{i}"), Span::call_site()))
                        .collect();

                    routes.push(quote! {
                        let mut route = #http_method({
                            let this = self.clone();
                            move |#(#args,)*| {
                                async move { Self::#ident(&this, #(#args,)*).await }
                            }
                        });
                        #(
                            let middleware = app
                                .get_component::<<#middleware as ::diode::Service>::Handle>()
                                .ok_or_else(|| {
                                    format!(
                                        "Missing component: {}",
                                        ::std::any::type_name::<<#middleware as ::diode::Service>::Handle>()
                                    )
                                })
                                .unwrap();
                            route = route.layer(::diode_http::MiddlewareLayerImpl(middleware));
                        )*
                        router = router.route(#path, route);
                    });
                }
                Err(e) => errors.push(e),
            }
        }
    }

    if !errors.is_empty() {
        let mut combined_error = Error::new(
            Span::call_site(),
            "Errors occurred while processing route attributes",
        );
        for error in errors {
            combined_error.combine(error);
        }
        return combined_error.to_compile_error().into();
    }

    quote! {
        #cleaned_input

        impl ::diode_http::RouterBuilder for #self_ty {
            fn build_router(self: ::std::sync::Arc<Self>, app: &::diode::App) -> ::diode_http::Router {
                let mut router = ::diode_http::Router::new();
                #(#routes)*
                #(
                    let middleware = app
                        .get_component::<<#router_middleware as ::diode::Service>::Handle>()
                        .ok_or_else(|| {
                            format!(
                                "Missing component: {}",
                                ::std::any::type_name::<<#router_middleware as ::diode::Service>::Handle>()
                            )
                        })
                        .unwrap();
                    router = router.layer(::diode_http::MiddlewareLayerImpl(middleware));
                )*
                router
            }
        }
    }
    .into()
}
