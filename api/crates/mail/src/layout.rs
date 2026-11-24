//! The look every message shares: the Stellarr lockup above one card on the
//! Midnight surfaces, the indigo accent only on the main action, and small
//! print below. Each method takes plain text and encodes it.
//!
//! Mail clients keep little of CSS, so the page is tables with inline styles,
//! and it loads nothing: the lockup and the stickers travel inside the
//! message as inline images, and the type falls back from Outfit and
//! JetBrains Mono, which only a few readers have installed, to the system's.
//! The page declares itself dark so that clients which repaint light mail for
//! a dark mode leave it alone. Classic Outlook ignores max-width, so a table
//! only it reads holds the column at 560 px there.

use stellarr_smtp::InlineImage;

use crate::html::{authority_of, encode};
use crate::images;

// Midnight, from the brand guide.
const DARKEST: &str = "#09090b";
const SURFACE: &str = "#18181b";
const BORDER: &str = "#292e39";
const TEXT: &str = "#f3f4f6";
const MUTED: &str = "#b9bec9";
const FAINT: &str = "#9aa0ad";
const ACCENT: &str = "#6f65d1";

const SANS: &str =
    "'Outfit',ui-sans-serif,system-ui,-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif";
const MONO: &str =
    "'JetBrains Mono',ui-monospace,SFMono-Regular,Menlo,Consolas,'Liberation Mono',monospace";

pub(crate) struct MailLayout {
    language: String,
    content: String,
    images: Vec<InlineImage>,
    footer: Vec<String>,
    preheader: String,
    instance: Option<String>,
    /// The bottom margin of the last block, which the card's own padding
    /// gives back so the space under a closing button matches the top.
    trailing: i32,
}

impl MailLayout {
    pub(crate) fn new(language: &str) -> MailLayout {
        MailLayout {
            language: language.to_owned(),
            content: String::new(),
            images: vec![images::lockup()],
            footer: Vec::new(),
            preheader: String::new(),
            instance: None,
            trailing: 0,
        }
    }

    /// Names the instance beside the lockup. Activity mail leaves it out on
    /// purpose.
    pub(crate) fn instance(mut self, name: &str) -> Self {
        // An instance that kept the default name would read "Stellarr" twice.
        self.instance = if name.trim().eq_ignore_ascii_case("Stellarr") {
            None
        } else {
            Some(name.to_owned())
        };
        self
    }

    /// The line a mail client shows next to the subject in the list; it is
    /// not visible in the message.
    pub(crate) fn preheader(mut self, text: &str) -> Self {
        self.preheader = text.to_owned();
        self
    }

    pub(crate) fn illustration(mut self, image: InlineImage) -> Self {
        // Decorative: the heading below says what the mascot shows.
        self.content.push_str(&format!(
            r#"<img src="cid:{}" width="112" height="112" alt="" style="display:block;width:112px;height:112px;border:0;margin:0 0 20px">"#,
            encode(&image.content_id)
        ));
        self.images.push(image);
        self.trailing = 20;
        self
    }

    pub(crate) fn heading(mut self, text: &str) -> Self {
        self.content.push_str(&format!(
            r#"<h1 style="margin:0 0 12px;font-family:{SANS};font-size:24px;line-height:32px;font-weight:600;letter-spacing:-0.01em;color:{TEXT}">{}</h1>"#,
            encode(text)
        ));
        self.trailing = 12;
        self
    }

    pub(crate) fn paragraph(mut self, text: &str) -> Self {
        self.content.push_str(&format!(
            r#"<p style="margin:0 0 16px;font-family:{SANS};font-size:15px;line-height:24px;color:{MUTED}">{}</p>"#,
            encode(text)
        ));
        self.trailing = 16;
        self
    }

    /// The main action. A table cell carries the colour, so clients that drop
    /// padding on links still show a button.
    pub(crate) fn button(mut self, url: &str, label: &str) -> Self {
        self.content.push_str(&format!(
            r#"<table role="presentation" cellpadding="0" cellspacing="0" border="0" style="margin:8px 0 24px"><tr><td bgcolor="{ACCENT}" style="background:{ACCENT};border-radius:8px"><a href="{}" target="_blank" style="display:inline-block;padding:12px 22px;font-family:{SANS};font-size:15px;line-height:20px;font-weight:600;color:#ffffff;text-decoration:none;border-radius:8px">{}</a></td></tr></table>"#,
            encode(url),
            encode(label)
        ));
        self.trailing = 24;
        self
    }

    /// The address of the button spelled out, for clients that do not open it.
    pub(crate) fn link_fallback(mut self, url: &str, label: &str) -> Self {
        let link = encode(url);
        self.content.push_str(&format!(
            r#"<p style="margin:0 0 4px;font-family:{SANS};font-size:13px;line-height:20px;color:{FAINT}">{}</p>"#,
            encode(label)
        ));
        self.content.push_str(&format!(
            r#"<p style="margin:0 0 24px;font-family:{MONO};font-size:12px;line-height:18px;word-break:break-all"><a href="{link}" target="_blank" style="color:{MUTED};text-decoration:underline">{link}</a></p>"#
        ));
        self.trailing = 24;
        self
    }

    /// A one-time code, large and spaced so it reads at a glance and copies
    /// as one word.
    pub(crate) fn code(mut self, code: &str) -> Self {
        // letter-spacing also follows the last digit, so the left padding is
        // larger by that much to keep the code centred.
        self.content.push_str(&format!(
            r#"<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="margin:8px 0 24px"><tr><td align="center" bgcolor="{DARKEST}" class="code" style="background:{DARKEST};border:1px solid {BORDER};border-radius:12px;padding:20px 12px 20px 22px;font-family:{MONO};font-size:32px;line-height:40px;font-weight:600;letter-spacing:0.3em;color:{TEXT}">{}</td></tr></table>"#,
            encode(code)
        ));
        self.trailing = 24;
        self
    }

    /// Labelled facts, one under another, so they hold on a narrow screen.
    pub(crate) fn details(mut self, rows: &[(String, String)]) -> Self {
        self.content.push_str(&format!(
            r#"<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" bgcolor="{DARKEST}" style="margin:4px 0 24px;background:{DARKEST};border:1px solid {BORDER};border-radius:12px;border-collapse:separate">"#
        ));
        for (index, (label, value)) in rows.iter().enumerate() {
            let rule = if index + 1 < rows.len() {
                format!("border-bottom:1px solid {BORDER};")
            } else {
                String::new()
            };
            self.content.push_str(&format!(
                r#"<tr><td style="{rule}padding:12px 16px"><div style="font-family:{SANS};font-size:12px;line-height:16px;color:{FAINT}">{}</div><div style="padding-top:2px;font-family:{SANS};font-size:15px;line-height:22px;font-weight:500;color:{TEXT}">{}</div></td></tr>"#,
                encode(label),
                encode(value)
            ));
        }
        self.content.push_str("</table>");
        self.trailing = 24;
        self
    }

    pub(crate) fn divider(mut self) -> Self {
        self.content.push_str(&format!(
            r#"<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="margin:8px 0 20px"><tr><td style="border-top:1px solid {BORDER};font-size:0;line-height:0;height:1px">&nbsp;</td></tr></table>"#
        ));
        self.trailing = 20;
        self
    }

    /// Small print inside the card: what to do if the mail was not expected.
    pub(crate) fn note(mut self, text: &str) -> Self {
        self.content.push_str(&format!(
            r#"<p style="margin:0;font-family:{SANS};font-size:13px;line-height:20px;color:{FAINT}">{}</p>"#,
            encode(text)
        ));
        self.trailing = 0;
        self
    }

    /// A line under the card.
    pub(crate) fn footer(mut self, text: &str) -> Self {
        self.footer.push(encode(text));
        self
    }

    /// The instance's address under the card, as the host alone.
    pub(crate) fn footer_link(mut self, url: &str) -> Self {
        if url.is_empty() {
            return self;
        }
        self.footer.push(format!(
            r#"<a href="{}" target="_blank" style="color:{FAINT};text-decoration:underline">{}</a>"#,
            encode(url),
            encode(&authority_of(url))
        ));
        self
    }

    /// The finished page and the images it shows, the lockup first.
    pub(crate) fn render(self, subject: &str) -> (String, Vec<InlineImage>) {
        let instance = match &self.instance {
            None => String::new(),
            Some(name) => format!(
                r#"<td class="instance" align="right" style="vertical-align:middle;padding-left:16px;font-family:{SANS};font-size:13px;line-height:18px;color:{FAINT}">{}</td>"#,
                encode(name)
            ),
        };
        let footer = if self.footer.is_empty() {
            String::new()
        } else {
            format!(
                r#"<tr><td style="padding:20px 8px 0;font-family:{SANS};font-size:12px;line-height:18px;color:{FAINT}">{}</td></tr>"#,
                self.footer.join("<br>")
            )
        };
        // Fillers after the preheader keep clients from running the body's
        // first words into the preview line.
        let filler = "&#847;&zwnj;&nbsp;".repeat(60);
        let bottom = (36 - self.trailing).max(12);
        let language = encode(&self.language);
        let title = encode(subject);
        let preheader = encode(&self.preheader);
        let lockup = encode(&images::lockup().content_id);
        let content = &self.content;

        let html = format!(
            r#"<!doctype html>
<html lang="{language}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="x-apple-disable-message-reformatting">
<meta name="color-scheme" content="dark">
<meta name="supported-color-schemes" content="dark">
<title>{title}</title>
<style>
:root {{ color-scheme: dark; supported-color-schemes: dark; }}
body {{ margin: 0; padding: 0; -webkit-text-size-adjust: 100%; }}
@media (max-width: 600px) {{
  .shell {{ padding: 24px 12px !important; }}
  .card {{ padding-top: 28px !important; padding-left: 20px !important; padding-right: 20px !important; }}
  .code {{ font-size: 28px !important; }}
}}
</style>
</head>
<body bgcolor="{DARKEST}" style="margin:0;padding:0;background:{DARKEST}">
<div style="display:none;max-height:0;max-width:0;overflow:hidden;opacity:0;mso-hide:all">{preheader}{filler}</div>
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" bgcolor="{DARKEST}" style="background:{DARKEST}">
<tr><td align="center" class="shell" style="padding:40px 16px">
<!--[if mso]><table role="presentation" width="560" align="center" cellpadding="0" cellspacing="0" border="0"><tr><td><![endif]-->
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="max-width:560px">
<tr><td style="padding:0 8px 24px">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0"><tr>
<td style="vertical-align:middle"><img src="cid:{lockup}" width="141" height="36" alt="Stellarr" style="display:block;width:141px;height:36px;border:0;font-family:{SANS};font-size:20px;font-weight:600;color:{TEXT}"></td>
{instance}
</tr></table>
</td></tr>
<tr><td class="card" bgcolor="{SURFACE}" style="background:{SURFACE};border:1px solid {BORDER};border-radius:16px;padding:36px 32px {bottom}px">
{content}
</td></tr>
{footer}
</table>
<!--[if mso]></td></tr></table><![endif]-->
</td></tr>
</table>
</body>
</html>"#
        );
        (html, self.images)
    }
}
