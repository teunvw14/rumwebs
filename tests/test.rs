use std::collections::HashMap;
use std::fs;

use rumwebs::HTTP;

#[test]
fn show_http_response() {
    let status = HTTP::StatusCodes::NOT_FOUND;
    let mut headers: HashMap<String, String> = HashMap::new();
    headers.insert("Content-Type".to_string(), "image/png".to_string());
    let body = fs::read("smile.png").unwrap();
    let resp = HTTP::Response::new()
        .with_status(status)
        .with_headers(headers)
        .with_body(body);
    assert_eq!(
        format!("{}", resp),
        r#"HTTP/1.1 404 Not Found\r\nContent-Type: image/png\r\n\r\n�PNG\r\n\u{1a}\n\u{0}\u{0}\u{0}\rIHDR\u{0}\u{0}\u{0}\u{1f}\u{0}\u{0}\u{0}\u{13}\u{8}\u{2}\u{0}\u{0}\u{0}�\u{e}A\u{15}\u{0}\u{0}\u{0}\u{1}sRGB\u{0}��\u{1c}�\u{0}\u{0}\u{0}\u{4}gAMA\u{0}\u{0}��\u{b}�a\u{5}\u{0}\u{0}\u{0}\tpHYs\u{0}\u{0}\u{12}t\u{0}\u{0}\u{12}t\u{1}�f\u{1f}x\u{0}\u{0}\u{0}�IDAT8O풱\r\u{3}!\u{c}E))))3\nc0\u{2}�0\u{6}c0\u{1a}%�b+g\u{1c}�t$�(R^g}�\u{c}¦_ɗ��\u{10}B)�j�\"\u{12}�v�1�ZK5c\u{11}\tT;�#T3(�E��]C=\u{1}��)\u{16}�@=�s���\u{12}Ռi\u{4}kz{�7����\u{14}��\u{0}�T����m\u{c}��a;��Dv|,rv\u{0}\u{1c}v�Q��\u{19}G�\u{7}l#>c\u{18}���\u{15}\u{18}��ހ\u{18}ck�\u{14}\u{c}i�,�k��\u{e}I\u{18}Yu��Z?\u{0}\u{0}\u{0}\u{0}IEND�B`�""#
    );
}
