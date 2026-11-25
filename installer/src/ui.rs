//! Everything the installer shows goes through `Ui`, so its look changes in
//! one place. Terminal mode has glyphs, colour and a redrawn waiting line;
//! plain mode, for logs and provisioning tools, has none of them.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, IsTerminal, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// The run stops with this exit status; what to say was said already.
#[derive(Debug)]
pub struct Stop(pub i32);

// What an interrupt or a fault has to tidy on the screen, kept where a
// signal handler can read it.
pub static TERMINAL: AtomicBool = AtomicBool::new(false);
pub static CURSOR_HIDDEN: AtomicBool = AtomicBool::new(false);
/// 0: nothing open, 1: a prompt waits for its answer, 2: the waiting line.
pub static OPEN: AtomicI32 = AtomicI32::new(0);
pub static ASK_FD: AtomicI32 = AtomicI32::new(1);
const OPEN_PROMPT: i32 = 1;
const OPEN_WAIT: i32 = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Fd {
    Out,
    Err,
}

pub fn emit(fd: Fd, text: &str) {
    // A closed output is no reason to stop; there is nobody to tell.
    match fd {
        Fd::Out => {
            let mut out = io::stdout().lock();
            let _ = out.write_all(text.as_bytes());
            let _ = out.flush();
        }
        Fd::Err => {
            let mut err = io::stderr().lock();
            let _ = err.write_all(text.as_bytes());
            let _ = err.flush();
        }
    }
}

fn env(name: &str) -> String {
    std::env::var_os(name)
        .map(|v| v.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub struct Ui {
    pub terminal: bool,
    utf8: bool,
    pub r: &'static str,
    pub b: &'static str,
    d: &'static str,
    green: &'static str,
    yellow: &'static str,
    red: &'static str,
    accent: &'static str,
    clear: &'static str,
    g_ok: &'static str,
    g_warn: &'static str,
    g_fail: &'static str,
    g_prompt: &'static str,
    g_bullet: &'static str,
    frames: &'static [&'static str],
    answered: bool,
    frame: usize,
    wait_next: u64,
    /// Where questions go when the output itself goes to a log.
    ask: Option<File>,
    /// Where answers come from, when the installer may ask.
    answers: Option<BufReader<File>>,
    answers_from_terminal: bool,
}

impl Ui {
    pub fn new(color: bool) -> Ui {
        // STELLARR_INSTALLER_UI forces one mode, for the tests and for screen
        // captures, where the output is not a terminal.
        let terminal = match env("STELLARR_INSTALLER_UI").as_str() {
            "terminal" => true,
            "plain" => false,
            _ => io::stdout().is_terminal() && !matches!(env("TERM").as_str(), "" | "dumb"),
        };
        let locale = [env("LC_ALL"), env("LC_CTYPE"), env("LANG")]
            .into_iter()
            .find(|v| !v.is_empty())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let utf8 = locale.contains("utf-8") || locale.contains("utf8");
        TERMINAL.store(terminal, Ordering::Relaxed);

        let mut ui = Ui {
            terminal,
            utf8,
            r: "",
            b: "",
            d: "",
            green: "",
            yellow: "",
            red: "",
            accent: "",
            clear: "",
            g_ok: "ok",
            g_warn: "!",
            g_fail: "x",
            g_prompt: ">",
            g_bullet: "-",
            frames: &["-", "\\", "|", "/"],
            answered: false,
            frame: 0,
            wait_next: 0,
            ask: None,
            answers: None,
            answers_from_terminal: false,
        };
        if terminal {
            ui.clear = "\x1b[K";
            if color && env("NO_COLOR").is_empty() {
                ui.r = "\x1b[0m";
                ui.b = "\x1b[1m";
                ui.d = "\x1b[2m";
                // The basic colours follow the terminal's own theme; the
                // accent is the brand's violet, #6f65d1, as near as the
                // terminal can show it.
                ui.green = "\x1b[32m";
                ui.yellow = "\x1b[33m";
                ui.red = "\x1b[31m";
                ui.accent = match env("COLORTERM").as_str() {
                    "truecolor" | "24bit" => "\x1b[38;2;111;101;209m",
                    _ if env("TERM").contains("256color") => "\x1b[38;5;62m",
                    _ => "\x1b[35m",
                };
            }
            if utf8 {
                ui.g_ok = "✓";
                ui.g_fail = "✗";
                ui.g_prompt = "›";
                ui.g_bullet = "·";
                ui.frames = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            }
        }
        ui
    }

    /// Opens where answers are read, `tty`, and decides whether the installer
    /// may ask. /dev/tty exists without a controlling terminal too; only
    /// opening it tells the difference.
    pub fn open_answers(&mut self, tty: &Path) -> bool {
        if !tty.exists() || OpenOptions::new().read(true).write(true).open(tty).is_err() {
            return false;
        }
        let Ok(file) = File::open(tty) else {
            return false;
        };
        self.answers_from_terminal = file.is_terminal();
        self.answers = Some(BufReader::new(file));
        // With the output going to a log, questions still go where the
        // person is.
        if self.answers_from_terminal
            && !io::stdout().is_terminal()
            && let Ok(ask) = OpenOptions::new().append(true).open(tty)
        {
            ASK_FD.store(ask.as_raw_fd(), Ordering::Relaxed);
            self.ask = Some(ask);
        }
        true
    }

    fn emit_ask(&mut self, text: &str) {
        match &mut self.ask {
            Some(file) => {
                let _ = file.write_all(text.as_bytes());
                let _ = file.flush();
            }
            None => emit(Fd::Out, text),
        }
    }

    pub fn banner(&mut self) {
        let (a, r, b, d) = (self.accent, self.r, self.b, self.d);
        if !self.terminal {
            emit(Fd::Out, "Stellarr installer\n");
        } else if !self.utf8 {
            emit(Fd::Out, &format!("\n  {b}Stellarr installer{r}\n"));
        } else {
            emit(
                Fd::Out,
                &format!(
                    "\n  ╭──────────────╮\n  │  {a}▂▂{r}  {a}██{r}  {a}▆▆{r}  │   {b}Stellarr{r}\n  │  {a}██{r}  {a}██{r}  {a}██{r}  │   \
{d}Self-hosted text and voice chat for small groups{r}\n  │  {a}▀▀{r}  {a}██{r}  {a}▀▀{r}  │\n  ╰─╮ ╭──────────╯\n    ╰─╯\n"
                ),
            );
        }
    }

    pub fn section(&mut self, title: &str) {
        self.settle();
        if self.terminal {
            emit(Fd::Out, &format!("\n  {}{title}{}\n", self.b, self.r));
        } else {
            emit(Fd::Out, &format!("==> {title}\n"));
        }
    }

    pub fn ok<S: AsRef<str>>(&mut self, text: &str, more: &[S]) {
        self.result(Fd::Out, self.green, self.g_ok, "[ok]", text, more);
    }

    pub fn warn<S: AsRef<str>>(&mut self, text: &str, more: &[S]) {
        self.result(Fd::Err, self.yellow, self.g_warn, "[warn]", text, more);
    }

    pub fn fail<S: AsRef<str>>(&mut self, text: &str, more: &[S]) {
        self.result(Fd::Err, self.red, self.g_fail, "[error]", text, more);
    }

    /// One result line: the glyph and the text, and any further lines under
    /// it. Plain mode puts the tag first and keeps it to one line. In a
    /// terminal a line right after an answer is set off from it by an empty
    /// line, as the next question would be.
    fn result<S: AsRef<str>>(
        &mut self,
        fd: Fd,
        colour: &str,
        glyph: &str,
        tag: &str,
        text: &str,
        more: &[S],
    ) {
        let after_answer = self.answered;
        self.settle();
        let mut out = String::new();
        if self.terminal {
            if after_answer {
                out.push('\n');
            }
            out.push_str(&format!("   {colour}{glyph}{} {text}\n", self.r));
            for line in more {
                out.push_str(&format!("     {}\n", line.as_ref()));
            }
        } else {
            out.push_str(&format!("{tag} {text}"));
            for line in more {
                out.push(' ');
                out.push_str(line.as_ref());
            }
            out.push('\n');
        }
        emit(fd, &out);
    }

    /// Lines of text; an empty one is an empty line, which plain mode leaves
    /// out. A line that starts with two spaces is a command to type and keeps
    /// them.
    pub fn text<S: AsRef<str>>(&mut self, fd: Fd, lines: &[S]) {
        self.settle();
        let mut out = String::new();
        for line in lines {
            let line = line.as_ref();
            if !self.terminal {
                if !line.is_empty() {
                    out.push_str(line);
                    out.push('\n');
                }
            } else if line.is_empty() {
                out.push('\n');
            } else {
                out.push_str("  ");
                out.push_str(line);
                out.push('\n');
            }
        }
        emit(fd, &out);
    }

    pub fn bullet<S: AsRef<str>>(&mut self, text: &str, more: &[S]) {
        self.settle();
        let mut out = String::new();
        if self.terminal {
            out.push_str(&format!("   {} {text}\n", self.g_bullet));
            for line in more {
                out.push_str(&format!("     {}\n", line.as_ref()));
            }
        } else {
            out.push_str(&format!("- {text}"));
            for line in more {
                out.push(' ');
                out.push_str(line.as_ref());
            }
            out.push('\n');
        }
        emit(Fd::Out, &out);
    }

    /// Pairs of a name and a value in two columns. With `strong` the values
    /// are bold: they are what the person has to read or copy.
    pub fn rows(&mut self, strong: bool, pairs: &[(&str, &str)]) {
        let (on, off) = if strong { (self.b, self.r) } else { ("", "") };
        let width = pairs.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
        self.settle();
        let mut out = String::new();
        for (name, value) in pairs {
            if self.terminal {
                out.push_str(&format!("    {name:<width$}   {on}{value}{off}\n"));
            } else {
                out.push_str(&format!("{name}: {value}\n"));
            }
        }
        emit(Fd::Out, &out);
    }

    /// Another program's output, indented. Its colours, cursor movements and
    /// carriage returns are taken out, so they cannot disturb the screen or a
    /// log.
    pub fn block(&mut self, fd: Fd, output: &[u8]) {
        self.settle();
        let text = String::from_utf8_lossy(output);
        let mut out = String::new();
        for line in text.split_terminator('\n') {
            out.push_str("    ");
            out.push_str(&strip_controls(line));
            out.push('\n');
        }
        emit(fd, &out);
    }

    pub fn question<S: AsRef<str>>(&mut self, title: &str, lines: &[S]) {
        self.settle();
        let mut out = String::new();
        if self.terminal {
            out.push_str(&format!("\n  {}{title}{}\n", self.b, self.r));
            for line in lines {
                out.push_str(&format!("  {}{}{}\n", self.d, line.as_ref(), self.r));
            }
        } else {
            out.push_str(title);
            out.push('\n');
            for line in lines {
                out.push_str(line.as_ref());
                out.push('\n');
            }
        }
        self.emit_ask(&out);
    }

    /// Says why an answer was not taken, where the question is.
    pub fn retry(&mut self, why: &str) {
        self.settle();
        let line = if self.terminal {
            format!("   {}{}{} {why}\n", self.yellow, self.g_warn, self.r)
        } else {
            format!("[warn] {why}\n")
        };
        self.emit_ask(&line);
    }

    /// Shows the prompt with the default in brackets and reads one line of
    /// answer, without surrounding blanks. When there is nothing more to
    /// read, the installer stops: no answer is ever assumed.
    pub fn prompt(&mut self, default: &str) -> Result<String, Stop> {
        let shown = if default.is_empty() {
            String::new()
        } else {
            format!("[{default}] ")
        };
        self.settle();
        let prompt = if self.terminal {
            format!("  {}{}{} {shown}", self.accent, self.g_prompt, self.r)
        } else {
            format!("{} {shown}", self.g_prompt)
        };
        self.emit_ask(&prompt);
        OPEN.store(OPEN_PROMPT, Ordering::Relaxed);
        let mut line = Vec::new();
        let read = match &mut self.answers {
            Some(answers) => answers.read_until(b'\n', &mut line).unwrap_or(0),
            None => 0,
        };
        if read == 0 {
            self.fail(
                "The terminal gave no answer, so the installer stopped",
                &[] as &[&str],
            );
            return Err(Stop(1));
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let line = String::from_utf8_lossy(&line).into_owned();
        // A terminal shows what was typed itself; answers from a file are
        // shown here, so the screen and a log read the same.
        if !self.answers_from_terminal {
            self.emit_ask(&format!("{line}\n"));
        }
        OPEN.store(0, Ordering::Relaxed);
        self.answered = true;
        Ok(line
            .replace('\r', "")
            .trim_matches(|c: char| c.is_ascii_whitespace() || c == '\x0b')
            .to_string())
    }

    /// A yes or no question; `default_yes` is what an empty answer means.
    pub fn confirm<S: AsRef<str>>(
        &mut self,
        default_yes: bool,
        title: &str,
        lines: &[S],
    ) -> Result<bool, Stop> {
        self.question(title, lines);
        loop {
            let answer = self.prompt(if default_yes { "Y/n" } else { "y/N" })?;
            match answer.to_ascii_lowercase().as_str() {
                "" => return Ok(default_yes),
                "y" | "yes" => return Ok(true),
                "n" | "no" => return Ok(false),
                _ => self.retry("Answer y or n"),
            }
        }
    }

    /// The line shown while Stellarr starts. A terminal redraws it on every
    /// check; a log gets it once and then every 30 seconds.
    pub fn wait(&mut self, elapsed: u64, limit: u64) {
        if self.terminal {
            if !CURSOR_HIDDEN.swap(true, Ordering::Relaxed) {
                emit(Fd::Out, "\x1b[?25l");
            }
            let frame = self.frames[self.frame % self.frames.len()];
            self.frame += 1;
            emit(
                Fd::Out,
                &format!(
                    "\r{}   {}{frame}{} Waiting for Stellarr to start, {elapsed} s",
                    self.clear, self.accent, self.r
                ),
            );
            OPEN.store(OPEN_WAIT, Ordering::Relaxed);
        } else if elapsed >= self.wait_next {
            if self.wait_next == 0 {
                emit(
                    Fd::Out,
                    &format!("Waiting for Stellarr to start, for up to {limit} s\n"),
                );
            } else {
                emit(Fd::Out, &format!("Still waiting, {elapsed} s\n"));
            }
            self.wait_next = elapsed + 30;
        }
    }

    /// Ends a waiting line or an unanswered prompt before anything else is
    /// shown.
    pub fn settle(&mut self) {
        match OPEN.swap(0, Ordering::Relaxed) {
            OPEN_WAIT => emit(Fd::Out, &format!("\r{}", self.clear)),
            OPEN_PROMPT => self.emit_ask("\n"),
            _ => {}
        }
        self.answered = false;
        if CURSOR_HIDDEN.swap(false, Ordering::Relaxed) {
            emit(Fd::Out, "\x1b[?25h");
        }
    }
}

/// `line` without escape sequences and carriage returns.
fn strip_controls(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {}
            '\x1b' => {
                if chars.peek() == Some(&'[') {
                    let mut ahead = chars.clone();
                    ahead.next();
                    while ahead
                        .peek()
                        .is_some_and(|c| c.is_ascii_digit() || *c == ';' || *c == '?')
                    {
                        ahead.next();
                    }
                    if ahead.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                        ahead.next();
                        chars = ahead;
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

pub fn usage() -> &'static str {
    "Installs Stellarr with Docker Compose, or updates an installation it made.

Usage:
  curl -fsSL https://stellarr.online/install.sh | bash -s -- [options]
  ./install.sh [options]                in a checkout

Where:
  --dir PATH           The installation's directory. Without it: the checkout
                       the script is in, the directory of the installation
                       Docker runs, the current directory when it holds
                       Stellarr's docker-compose.yml, or ~/stellarr.

Settings, kept in .env (a terminal asks for the first three):
  --domain NAME        Domain or IPv4 address people open (default localhost)
  --public-ip IP       IPv4 address voice calls use, or \"auto\" to resolve
                       --domain (default: none, voice on this machine only)
  --language LANG      Interface language, en or pl (default en)
  --https-port PORT    HTTPS port on this machine (default 443)
  --public-port PORT   HTTPS port in links, 443 behind a proxy (default: the
                       HTTPS port)
  --http-port BIND     HTTP bind (default 127.0.0.1:8888)
  --media-port PORT    UDP and TCP port of voice and video (default 49000)
  --enable-turn        Publish optional TURN and its relay ports
  --disable-turn       Disable TURN, including on upgrades
  --turn-port PORT     Optional STUN and TURN host port (default 3478)
  --turns-port PORT    Optional TURN TLS and DTLS host port (default 5349)

Release:
  --version TAG        Image to run: a release such as 1.4.0, or a moving tag
                       such as latest (default: the latest release, pinned)
  --ref REF            Git tag or branch to take docker-compose.yml from
                       (default: the release's tag)
  --build              Build the image from this checkout instead of pulling

Behaviour:
  -y, --yes            Ask nothing: take the defaults and confirm updates
  --no-start           Write the files and check the ports, but start nothing
  --install-docker     Install Docker Engine from https://get.docker.com when
                       it is missing (Linux)
  --force              Back up .env and write a new one from the options and
                       defaults; take over a container whose directory is gone
  --skip-port-check    Do not check that the ports are free
  --no-color           No colour, as with a non-empty NO_COLOR
  -h, --help           Show this help

Each setting can come from the environment too: STELLARR_INSTALL_DIR,
STELLARR_DOMAIN, STELLARR_PUBLIC_IP, STELLARR_LANGUAGE, STELLARR_HTTPS_PORT,
STELLARR_PUBLIC_PORT, STELLARR_HTTP_PORT, STELLARR_MEDIA_PORT,
STELLARR_TURN_ENABLED, STELLARR_TURN_PORT, STELLARR_TURNS_PORT, STELLARR_VERSION, STELLARR_REF.
STELLARR_START_TIMEOUT is how long to wait for the start (default 300 s).

Running the installer again updates Stellarr to the latest release. It owns
docker-compose.yml and replaces it on an update, keeping the old one beside
it; local changes belong in .env or in docker-compose.override.yml.
"
}

#[cfg(test)]
mod tests {
    use super::strip_controls;

    #[test]
    fn strips_escape_sequences() {
        assert_eq!(strip_controls("\x1b[1;32mready\x1b[0m\r"), "ready");
        assert_eq!(strip_controls("\x1b[?25lx\x1b"), "x");
        assert_eq!(strip_controls("plain"), "plain");
    }
}
