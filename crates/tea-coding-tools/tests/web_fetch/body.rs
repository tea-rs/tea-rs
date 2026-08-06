use std::io::Write as _;

use flate2::Compression;
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use tea_coding_tools::{
    FetchBodyDecoder, FetchBodyLimits, FetchContentKind, FetchHttpHeaders, FetchProviderErrorCode,
    FetchTruncationReason, MAX_FETCH_DECODED_BYTES,
};

fn headers(content_type: Option<&str>, content_encoding: Option<&str>) -> FetchHttpHeaders {
    FetchHttpHeaders::new(
        content_type.map(str::to_owned),
        content_encoding.map(str::to_owned),
        None,
    )
    .unwrap()
}

fn gzip(value: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(value).unwrap();
    encoder.finish().unwrap()
}

fn zlib(value: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(value).unwrap();
    encoder.finish().unwrap()
}

fn raw_deflate(value: &[u8]) -> Vec<u8> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(value).unwrap();
    encoder.finish().unwrap()
}

fn brotli(value: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    {
        let mut encoder = brotli::CompressorWriter::new(&mut output, 4096, 5, 20);
        encoder.write_all(value).unwrap();
    }
    output
}

#[test]
fn body_limits_reject_invalid_configurations() {
    assert!(FetchBodyLimits::new(0, 1, 1).is_err());
    assert!(FetchBodyLimits::new(MAX_FETCH_DECODED_BYTES + 1, 1, 1).is_err());
    assert!(FetchBodyLimits::new(8, 9, 1).is_err());
    assert!(FetchBodyLimits::new(8, 8, 0).is_err());
}

#[test]
fn plain_text_is_decoded_normalized_and_truncated_by_character() {
    let decoded = FetchBodyDecoder::default()
        .decode(
            &headers(Some("text/plain; charset=utf-8"), None),
            "  你好a\r\nb\0".trim_end_matches('\0').as_bytes(),
            3,
        )
        .unwrap();

    assert_eq!(decoded.kind(), FetchContentKind::PlainText);
    assert_eq!(decoded.mime_type(), "text/plain; charset=utf-8");
    assert_eq!(decoded.body(), "你好a");
    assert_eq!(
        decoded.truncation(),
        Some(FetchTruncationReason::BodyCharacters)
    );
    assert_eq!(decoded.decoded_bytes(), "  你好a\r\nb".len());
    let debug = format!("{decoded:?}");
    assert!(debug.contains("body_chars: 3"));
    assert!(!debug.contains("你好a"));
}

#[test]
fn html_extraction_removes_inert_content_and_normalizes_title() {
    let html = br"<!doctype html><html><head>
        <title> Tea
        reference </title><style>.hidden { display: none }</style>
        </head><body>Hello <script>secret()</script><p>World</p>
        <template>template secret</template><svg><text>svg secret</text></svg>
        </body></html>";
    let decoded = FetchBodyDecoder::default()
        .decode(&headers(Some("text/html"), None), html, 1_000)
        .unwrap();

    assert_eq!(decoded.kind(), FetchContentKind::Html);
    assert_eq!(decoded.title(), Some("Tea reference"));
    assert!(decoded.body().contains("Hello"));
    assert!(decoded.body().contains("World"));
    assert!(!decoded.body().contains("Tea reference"));
    assert!(!decoded.body().contains("secret"));
    assert!(!decoded.body().contains("hidden"));
}

#[test]
fn missing_mime_is_sniffed_within_a_bounded_prefix() {
    let html = FetchBodyDecoder::default()
        .decode(&headers(None, None), b"  <!doctype html><p>Tea</p>", 100)
        .unwrap();
    assert_eq!(html.kind(), FetchContentKind::Html);
    assert!(html.body().contains("Tea"));

    let json = FetchBodyDecoder::default()
        .decode(&headers(None, None), br#" {"tea":true} "#, 100)
        .unwrap();
    assert_eq!(json.kind(), FetchContentKind::Json);
    assert_eq!(json.body(), r#"{"tea":true}"#);
}

#[test]
fn unsupported_or_binary_content_is_rejected() {
    let unsupported = FetchBodyDecoder::default()
        .decode(&headers(Some("image/png"), None), b"not an image", 100)
        .unwrap_err();
    assert_eq!(unsupported.code(), FetchProviderErrorCode::UnsupportedMime);

    let binary = FetchBodyDecoder::default()
        .decode(&headers(None, None), b"text\0binary", 100)
        .unwrap_err();
    assert_eq!(binary.code(), FetchProviderErrorCode::UnsupportedMime);
}

#[test]
fn gzip_zlib_raw_deflate_and_brotli_are_decoded() {
    let value = b"compressed tea response";
    for (encoding, compressed) in [
        ("gzip", gzip(value)),
        ("deflate", zlib(value)),
        ("deflate", raw_deflate(value)),
        ("br", brotli(value)),
    ] {
        let decoded = FetchBodyDecoder::default()
            .decode(
                &headers(Some("text/plain"), Some(encoding)),
                &compressed,
                100,
            )
            .unwrap();
        assert_eq!(decoded.body(), "compressed tea response");
        assert_eq!(decoded.decoded_bytes(), value.len());
    }
}

#[test]
fn decompression_and_parser_limits_fail_closed() {
    let decoder = FetchBodyDecoder::new(FetchBodyLimits::new(32, 32, 2).unwrap());
    let too_large = decoder
        .decode(
            &headers(Some("text/plain"), Some("gzip")),
            &gzip(&[b'a'; 64]),
            100,
        )
        .unwrap_err();
    assert_eq!(too_large.code(), FetchProviderErrorCode::ResponseTooLarge);

    let too_complex = decoder
        .decode(
            &headers(Some("text/html"), None),
            b"<html><body><p>x</p></body></html>",
            100,
        )
        .unwrap_err();
    assert_eq!(too_complex.code(), FetchProviderErrorCode::ResponseTooLarge);
}

#[test]
fn charset_and_structured_content_are_validated() {
    let windows = FetchBodyDecoder::default()
        .decode(
            &headers(Some("text/plain; charset=windows-1252"), None),
            &[0x80],
            100,
        )
        .unwrap();
    assert_eq!(windows.body(), "€");

    for (content_type, body, expected) in [
        (
            "text/plain; charset=not-a-charset",
            b"a".as_slice(),
            FetchProviderErrorCode::UnsupportedMime,
        ),
        (
            "text/plain; charset=utf-8",
            &[0xff][..],
            FetchProviderErrorCode::MalformedResponse,
        ),
        (
            "application/json",
            br#"{"tea":}"#,
            FetchProviderErrorCode::MalformedResponse,
        ),
    ] {
        let error = FetchBodyDecoder::default()
            .decode(&headers(Some(content_type), None), body, 100)
            .unwrap_err();
        assert_eq!(error.code(), expected);
    }
}
