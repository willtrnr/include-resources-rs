use inflector::Inflector;
use proc_macro2::Literal;
use quote::{format_ident, quote};
use std::{
    io,
    path::{Path, PathBuf},
};
use syn::{Ident, parse_quote};

#[non_exhaustive]
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Ui {
    ImGui,
    Iced,
}

#[derive(Clone, Debug)]
pub struct Config {
    source_dir: PathBuf,
    ui: Option<Ui>,
}

impl Config {
    pub fn new(source_dir: impl Into<PathBuf>) -> Self {
        Self {
            source_dir: source_dir.into(),
            ui: None,
        }
    }

    pub fn with_ui(self, value: Ui) -> Self {
        let mut slf = self;
        slf.ui = Some(value);
        slf
    }

    pub fn with_imgui(self) -> Self {
        let mut slf = self;
        slf.ui = Some(Ui::ImGui);
        slf
    }

    pub fn with_iced(self) -> Self {
        let mut slf = self;
        slf.ui = Some(Ui::Iced);
        slf
    }
}

#[allow(unused)]
#[derive(Debug)]
struct Resource<'a> {
    path: &'a Path,
    name: &'a str,
    stem: &'a str,
    ext: &'a str,
    ident: Ident,
    data: proc_macro2::TokenStream,
}

type ResourceHandler = Box<dyn Fn(&Config, Resource<'_>) -> proc_macro2::TokenStream>;

pub fn generate(config: &Config) -> io::Result<String> {
    let resources = config
        .source_dir
        .read_dir()?
        .filter_map(Result::ok)
        .filter_map(|e| {
            if e.metadata().is_ok_and(|m| m.is_file()) {
                let file_name = e.file_name();
                let path = e.path();

                let name = file_name.to_str()?;

                let ident_data = format_ident!("{}", name.to_screaming_snake_case());
                let path_lit = Literal::string(e.path().to_str()?);

                let data = quote! {
                    #[allow(unused)]
                    pub const #ident_data: &[u8] = include_bytes!(#path_lit);
                };

                Some(
                    if let Some(ext) = e.path().extension().and_then(|v| v.to_str())
                        && let Some(handler) = get_handler(ext.to_ascii_lowercase())
                    {
                        handler(
                            config,
                            Resource {
                                path: path.as_path(),
                                name,
                                stem: path.file_stem().and_then(|v| v.to_str()).unwrap_or(name),
                                ext,
                                ident: ident_data,
                                data,
                            },
                        )
                    } else {
                        data
                    },
                )
            } else {
                None
            }
        });

    Ok(prettyplease::unparse(&parse_quote! { #(#resources)* }))
}

fn get_handler(ext: impl AsRef<str>) -> Option<ResourceHandler> {
    Some(match ext.as_ref() {
        "bmp" | "ico" | "jpeg" | "jpg" | "png" | "tga" | "webp" => Box::new(handle_img),
        #[cfg(feature = "fonts")]
        "otf" | "ttf" => Box::new(handle_font),
        _ => return None,
    })
}

fn handle_img(config: &Config, res: Resource<'_>) -> proc_macro2::TokenStream {
    let mut data = res.data;
    let ident_data = res.ident;

    let ident = format_ident!("{}", res.stem.to_screaming_snake_case());

    let format = format_ident!(
        "{}",
        match res.ext.to_ascii_lowercase().as_str() {
            "jpg" => "Jpeg".to_owned(),
            "webp" => "WebP".to_owned(),
            ext => ext.to_pascal_case(),
        }
    );

    data.extend(quote! {
        #[allow(unused)]
        pub static #ident: ::std::sync::LazyLock<::image::RgbaImage> =
            ::std::sync::LazyLock::new(|| {
                ::image::load_from_memory_with_format(#ident_data, ::image::ImageFormat::#format)
                    .expect("valid image data")
                    .into_rgba8()
            });
    });

    if let Some(Ui::Iced) = config.ui {
        let ident_handle = format_ident!("{}_H", ident);

        data.extend(quote! {
            #[allow(unused)]
            pub static #ident_handle: ::std::sync::LazyLock<::iced::advanced::image::Handle> =
                ::std::sync::LazyLock::new(|| {
                    let img = &*#ident;
                    let (width, height) = image::GenericImageView::dimensions(img);
                    ::iced::advanced::image::Handle::from_rgba(width, height, img.to_vec())
                });
        });
    }

    data
}

#[cfg(feature = "fonts")]
fn handle_font(config: &Config, res: Resource<'_>) -> proc_macro2::TokenStream {
    fn parse_family(path: &Path) -> Option<(String, Option<String>, Option<String>)> {
        use read_fonts::types::NameId;
        use read_fonts::{FontRef, TableProvider};
        use std::fs;

        let buf = fs::read(path).ok()?;
        let font = FontRef::new(&buf).ok()?;

        let mut name = None;
        let mut family = None;
        let mut sub_family = None;

        let table = font.name().ok()?;
        for rec in table.name_record() {
            match rec.name_id() {
                NameId::FULL_NAME => {
                    name = Some(rec.string(table.string_data()).ok()?.to_string());
                }
                NameId::FAMILY_NAME => {
                    family = Some(rec.string(table.string_data()).ok()?.to_string());
                }
                NameId::SUBFAMILY_NAME => {
                    sub_family = Some(rec.string(table.string_data()).ok()?.to_string());
                }
                _ => {}
            }
        }

        name.map(|n| (n, family, sub_family))
    }

    let mut data = res.data;
    let ident_data = res.ident;

    match config.ui {
        Some(Ui::ImGui) => {
            let ident = format_ident!("{}", res.stem.to_snake_case());
            let name_lit = Literal::string(res.name);

            data.extend(quote! {
                pub mod #ident {
                    #[allow(unused)]
                    pub fn font_source(size: f32) -> imgui::FontSource<'static> {
                        ::imgui::FontSource::TtfData {
                            data: super::#ident_data,
                            size_pixels: size,
                            config: Some(::imgui::FontConfig {
                                size_pixels: size,
                                name: Some(#name_lit.to_owned()),
                                ..Default::default()
                            }),
                        }
                    }
                }
            });
        }
        Some(Ui::Iced) => {
            if let Some((name, family, sub_family)) = parse_family(res.path) {
                let ident = format_ident!("{}", name.to_screaming_snake_case());

                let family = Literal::string(&family.unwrap_or(name));

                data.extend(
                    if let Some(sub_family) = sub_family
                        && !sub_family.trim().is_empty()
                    {
                        let subs = sub_family
                            .split_ascii_whitespace()
                            .filter_map(|s| Some(match s.to_lowercase().as_str() {
                                "black" => quote! { weight: ::iced::font::Weight::Black, },
                                "bold" => quote! { weight: ::iced::font::Weight::Bold, },
                                "italic" => quote! { style: ::iced::font::Style::Italic, },
                                "light" => quote! { weight: ::iced::font::Weight::Light, },
                                "medium" => quote! { weight: ::iced::font::Weight::Medium, },
                                "oblique" => quote! { style: ::iced::font::Style::Oblique, },
                                "regular" => quote! { style: ::iced::font::Style::Normal, weight: ::iced::font::Weight::Normal, },
                                "thin" => quote! { weight: ::iced::font::Weight::Thin, },
                                _ => return None,
                            }));

                        quote! {
                            #[allow(unused)]
                            pub const #ident: ::iced::Font = ::iced::Font {
                                #(#subs)*
                                ..::iced::Font::with_name(#family)
                            };
                        }
                    } else {
                        quote! {
                            #[allow(unused)]
                            pub const #ident: ::iced::Font = ::iced::Font::with_name(#family);
                        }
                    },
                );
            }
        }
        _ => {}
    }

    data
}
