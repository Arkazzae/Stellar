//! The kinds of message the application sends, built the way its templates
//! build them: one long HTML page in tables with inline styles, a text
//! twin, a lockup image and sometimes a sticker, Polish words everywhere.
//! The templates themselves live elsewhere; these are stand-ins of the same
//! shape and size.

#![allow(dead_code)]

use std::fs;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use stellarr_smtp::{InlineImage, Mailbox, Message};

pub struct Sample {
    pub name: &'static str,
    pub message: Message,
}

/// Bytes that look like a PNG to anything that checks the signature and
/// otherwise like noise, so that every byte value occurs.
fn picture(seed: u32, length: usize) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut state = seed;
    while out.len() < length {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        out.push((state >> 8) as u8);
    }
    out
}

fn image(id: &str, file: &str, seed: u32, length: usize) -> InlineImage {
    InlineImage {
        content_id: id.into(),
        file_name: file.into(),
        content_type: "image/png".into(),
        content: picture(seed, length),
    }
}

fn page(
    subject: &str,
    preheader: &str,
    heading: &str,
    paragraphs: &[&str],
    button: Option<(&str, &str)>,
    sticker: bool,
) -> String {
    let filler = "&#847;&zwnj;&nbsp;".repeat(60);
    let mut body = String::new();
    if sticker {
        body.push_str(r#"<img src="cid:mention@stellarr" width="112" height="112" alt="" style="display:block;width:112px;height:112px;border:0;margin:0 0 20px">"#);
    }
    body.push_str(&format!(r#"<h1 style="margin:0 0 16px;font-family:Outfit,'Segoe UI',Arial,sans-serif;font-size:24px;line-height:30px;color:#f1f1f7">{heading}</h1>"#));
    for p in paragraphs {
        body.push_str(&format!(r#"<p style="margin:0 0 16px;font-family:Outfit,'Segoe UI',Arial,sans-serif;font-size:15px;line-height:24px;color:#c9c9d6">{p}</p>"#));
    }
    if let Some((url, label)) = button {
        body.push_str(&format!(r##"<table role="presentation" cellpadding="0" cellspacing="0" border="0" style="margin:8px 0 24px"><tr><td bgcolor="#6f65d1" style="background:#6f65d1;border-radius:8px"><a href="{url}" target="_blank" style="display:inline-block;padding:12px 22px;font-size:15px;line-height:20px;font-weight:600;color:#ffffff;text-decoration:none;border-radius:8px">{label}</a></td></tr></table>"##));
    }
    format!(
        "<!doctype html>\n<html lang=\"pl\">\n<head>\n<meta charset=\"utf-8\">\n<title>{subject}</title>\n<style>\n:root {{ color-scheme: dark; }}\n@media (max-width: 600px) {{\n  .shell {{ padding: 24px 12px !important; }}\n}}\n</style>\n</head>\n<body bgcolor=\"#0b0b12\" style=\"margin:0;padding:0;background:#0b0b12\">\n<div style=\"display:none;max-height:0;overflow:hidden;opacity:0\">{preheader}{filler}</div>\n<table role=\"presentation\" width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" bgcolor=\"#0b0b12\"><tr><td align=\"center\" class=\"shell\" style=\"padding:40px 16px\">\n<!--[if mso]><table role=\"presentation\" width=\"560\" align=\"center\"><tr><td><![endif]-->\n<table role=\"presentation\" width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"max-width:560px\">\n<tr><td style=\"padding:0 8px 24px\"><img src=\"cid:lockup@stellarr\" width=\"141\" height=\"36\" alt=\"Stellarr\" style=\"display:block;width:141px;height:36px;border:0\"></td></tr>\n<tr><td class=\"card\" bgcolor=\"#15151f\" style=\"background:#15151f;border:1px solid #2a2a3a;border-radius:16px;padding:36px 32px 12px\">\n{body}\n</td></tr>\n</table>\n<!--[if mso]></td></tr></table><![endif]-->\n</td></tr></table>\n</body>\n</html>\n"
    )
}

fn base(
    name: &'static str,
    to: &str,
    subject: &str,
    text: &str,
    html: String,
    images: Vec<InlineImage>,
) -> Sample {
    Sample {
        name,
        message: Message {
            from: Mailbox::parse("Stellarr Orbit <noreply@chat.example.test>").unwrap(),
            to: vec![Mailbox::parse(to).unwrap()],
            subject: subject.into(),
            html,
            text: text.into(),
            message_id: format!("{name}/0123456789abcdef0123456789abcdef@stellarr"),
            date: UNIX_EPOCH + Duration::from_secs(1_795_000_000),
            images,
        },
    }
}

pub fn samples() -> Vec<Sample> {
    let lockup = || image("lockup@stellarr", "lockup.png", 1, 8453);
    let sticker = || image("mention@stellarr", "mention.png", 2, 34945);
    let dm = || image("direct-message@stellarr", "direct-message.png", 3, 42556);
    let url = "https://chat.example.test/reset-password?token=abc_DEF-123&x=1&y=zażółć";
    let mut all = Vec::new();

    let subject = "Zmień hasło w Orbit & Co";
    all.push(base(
        "password-reset",
        "jo@example.test",
        subject,
        &format!("Cześć Józefie,\n\nAby ustawić nowe hasło w Orbit & Co, otwórz ten link:\n{url}\n\nLink działa przez godzinę. Jeśli to nie Ty, zignoruj tę wiadomość.\n\n-- \nOrbit & Co · https://chat.example.test\n"),
        page(subject, "Ktoś poprosił o zmianę hasła", "Zmień hasło", &["Cześć Józefie, ktoś poprosił o zmianę hasła do Twojego konta.", "Jeśli to nie Ty, zignoruj tę wiadomość: nic się nie zmieni."], Some((url, "Zmień hasło")), false),
        vec![lockup()],
    ));

    let subject = "Potwierdź nowy adres e-mail w Orbit & Co";
    all.push(base(
        "email-change",
        "\"Łukasz Żółć\" <lukasz@example.test>",
        subject,
        &format!("Potwierdź adres, otwierając link:\n{url}\n"),
        page(
            subject,
            "Potwierdź adres",
            "Potwierdź nowy adres",
            &["Otwórz link, aby potwierdzić, że ta skrzynka jest Twoja."],
            Some((url, "Potwierdź adres")),
            false,
        ),
        vec![lockup()],
    ));

    let subject = "Stellarr: test of mail delivery";
    all.push(base(
        "delivery-test",
        "admin@example.test",
        subject,
        "This message proves the server can send mail.\n",
        page(
            subject,
            "Mail delivery works",
            "Mail delivery works",
            &["This message proves the server can send mail."],
            None,
            false,
        ),
        vec![lockup()],
    ));

    let subject = "Twój kod: 482 913";
    all.push(base(
        "email-code",
        "jo@example.test",
        subject,
        "Twój kod weryfikacyjny: 482913\nWażny przez 10 minut.\n",
        page(subject, "Kod weryfikacyjny", "Twój kod", &[r#"<span class="code" style="font-size:36px;letter-spacing:6px">482&nbsp;913</span>"#, "Ważny przez 10 minut."], None, false),
        vec![lockup()],
    ));

    let subject = "Nowe logowanie na Twoje konto w Orbit & Co z urządzenia Chrome na macOS, Warszawa (Polska)";
    all.push(base(
        "security-alert",
        "jo@example.test",
        subject,
        "Zalogowano się na Twoje konto.\nGdzie: Warszawa, Polska\nKiedy: 2026-11-18 11:06 UTC\nJeśli to nie Ty, zmień hasło.\n",
        page(subject, "Logowanie z nowego urządzenia", "Nowe logowanie", &["Zalogowano się na Twoje konto.", "Gdzie: Warszawa, Polska · Kiedy: 2026-11-18 11:06 UTC"], Some((url, "To nie ja")), false),
        vec![lockup()],
    ));

    let subject = "Zażółć wspomniał o Tobie w #ogólny";
    all.push(base(
        "mention",
        "jo@example.test",
        subject,
        "Zażółć napisał w #ogólny:\n> Cześć @jo, zerknij na to.\n.\n..kropki na początku linii\n",
        page(
            subject,
            "Zażółć wspomniał o Tobie",
            "Zażółć wspomniał o Tobie",
            &["„Cześć @jo, zerknij na to.”"],
            Some(("https://chat.example.test/channels/1", "Otwórz")),
            true,
        ),
        vec![lockup(), sticker()],
    ));

    let subject = "Ala, Ola i „Ela” Kowalska: nowa wiadomość";
    let mut direct = base(
        "direct-message",
        "Zażółć Gęślą <zg@example.test>",
        subject,
        "Nowa wiadomość prywatna od Ali, Oli i Eli.\n",
        page(
            subject,
            "Nowa wiadomość prywatna",
            "Nowa wiadomość",
            &[
                "Ala, Ola i „Ela” Kowalska napisały do Ciebie.",
                r#"<img src="cid:direct-message@stellarr" width="112" height="112" alt="">"#,
            ],
            None,
            false,
        ),
        vec![lockup(), dm()],
    );
    direct.message.from = Mailbox::parse("Orbit, Co \"Ltd\" <noreply@chat.example.test>").unwrap();
    all.push(direct);

    // The edges: text only, HTML only, no trailing newline, trailing blanks,
    // tabs, bare CR and LF, one enormous line, and a line that is a dot.
    all.push(base(
        "text-only",
        "jo@example.test",
        "Tylko tekst",
        "Linia ze spacją na końcu \nLinia z tabem\t\n\nZażółć\rbare cr\n.\n",
        String::new(),
        vec![],
    ));
    all.push(base(
        "html-only",
        "jo@example.test",
        "Tylko HTML",
        "",
        "<p>Zażółć gęślą jaźń</p>".into(),
        vec![],
    ));
    all.push(base(
        "one-huge-line",
        "jo@example.test",
        "Jedna ogromna linia",
        "krótki tekst",
        format!(
            "<img src=\"cid:lockup@stellarr\" alt=\"\"><p>{}</p>",
            "zażółć gęślą jaźń, ".repeat(3000)
        ),
        vec![lockup()],
    ));
    all.push(base(
        "no-newline",
        "jo@example.test",
        "x",
        "no newline at the end",
        "<b>no newline</b>".into(),
        vec![],
    ));
    all
}

/// What a reader of the message must find, in the form the Python checks
/// read: one `key`, then hex of the value, per line.
pub fn write_expectations(sample: &Sample, path: &Path) {
    let hex = |s: &[u8]| s.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let m = &sample.message;
    let mut out = String::new();
    let mut line = |key: &str, values: &[&[u8]]| {
        out.push_str(key);
        for v in values {
            out.push('\t');
            out.push_str(&hex(v));
        }
        out.push('\n');
    };
    line("subject", &[m.subject.as_bytes()]);
    line(
        "from",
        &[
            m.from.name().unwrap_or("").as_bytes(),
            m.from.address().as_bytes(),
        ],
    );
    for to in &m.to {
        line(
            "to",
            &[to.name().unwrap_or("").as_bytes(), to.address().as_bytes()],
        );
    }
    line("message-id", &[m.message_id.as_bytes()]);
    line("date", &[b"1795000000"]);
    line("text", &[m.text.as_bytes()]);
    line("html", &[m.html.as_bytes()]);
    for i in &m.images {
        line(
            "image",
            &[
                i.content_id.as_bytes(),
                i.file_name.as_bytes(),
                i.content_type.as_bytes(),
                &i.content,
            ],
        );
    }
    fs::write(path, out).unwrap();
}
