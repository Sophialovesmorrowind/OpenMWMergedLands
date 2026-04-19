use std::borrow::Cow;
use std::env;

const RESET: &str = "\x1b[0m";

fn style<'a>(text: impl Into<Cow<'a, str>>, codes: &[&str]) -> String {
    let text = text.into();
    if env::var_os("NO_COLOR").is_some() {
        return text.into_owned();
    }

    let prefix = codes.join(";");
    format!("\x1b[{prefix}m{text}{RESET}")
}

pub fn bold<'a>(text: impl Into<Cow<'a, str>>) -> String {
    style(text, &["1"])
}

pub fn yellow<'a>(text: impl Into<Cow<'a, str>>) -> String {
    style(text, &["33"])
}

pub fn bold_red<'a>(text: impl Into<Cow<'a, str>>) -> String {
    style(text, &["1", "31"])
}
