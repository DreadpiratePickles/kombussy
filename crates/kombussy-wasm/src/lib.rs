//! WebAssembly surface for kombussy.
//!
//! The browser hands us bytes and a target format name; everything else stays
//! in `kombussy-core`. Errors cross the boundary as JavaScript `Error` values
//! carrying the codec's own message, so the UI never has to guess.

#![forbid(unsafe_code)]

use kombussy_core::{convert, decode, detect, Format};
use wasm_bindgen::prelude::*;

fn parse_format(name: &str) -> Result<Format, JsValue> {
    match name {
        "ttf" | "otf" | "sfnt" => Ok(Format::Sfnt),
        "woff" | "woff1" => Ok(Format::Woff1),
        "woff2" => Ok(Format::Woff2),
        other => Err(JsValue::from_str(&format!(
            "unknown target format '{other}'"
        ))),
    }
}

/// Convert a font to `target` ("ttf", "otf", "woff" or "woff2").
#[wasm_bindgen]
pub fn convert_font(input: &[u8], target: &str) -> Result<Vec<u8>, JsValue> {
    let format = parse_format(target)?;
    convert(input, format).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Report the container format of `input` without fully decoding it.
#[wasm_bindgen]
pub fn detect_format(input: &[u8]) -> Result<String, JsValue> {
    let format = detect(input).map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(match format {
        Format::Sfnt => "sfnt",
        Format::Woff1 => "woff",
        Format::Woff2 => "woff2",
    }
    .to_string())
}

/// The extension a conversion to `target` should produce for this input,
/// resolving the TTF/OTF split from the font's own outline flavor.
#[wasm_bindgen]
pub fn output_extension(input: &[u8], target: &str) -> Result<String, JsValue> {
    let format = parse_format(target)?;
    let font = decode(input).map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(format.extension(font.flavor).to_string())
}

/// Newline-separated `tag\tbyte_length` rows, for the table listing in the UI.
#[wasm_bindgen]
pub fn table_report(input: &[u8]) -> Result<String, JsValue> {
    let font = decode(input).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let mut rows: Vec<String> = font
        .tables
        .iter()
        .map(|t| format!("{}\t{}", String::from_utf8_lossy(&t.tag), t.data.len()))
        .collect();
    rows.sort();
    Ok(rows.join("\n"))
}
