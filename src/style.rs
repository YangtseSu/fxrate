// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Yangtse Su
//
// Terminal styling helpers. owo-colors builds the styled strings; whether
// styling is applied is decided per stream (interactive terminal without
// NO_COLOR) so pipes, redirected output, and CI never receive escape
// sequences. The chart additionally styles runs of canvas characters with
// literal SGR constants in `render` — same standard codes, chosen there
// because the run-wrapper needs bare prefix/suffix strings.

use std::fmt::Display;
use std::io::IsTerminal;

use owo_colors::OwoColorize;

/// Whether stdout should be styled: an interactive terminal with `NO_COLOR`
/// unset or empty.
pub fn stdout_color() -> bool {
    stream_color(std::io::stdout().is_terminal())
}

/// Whether stderr should be styled. Decided independently of stdout: a
/// piped stdout must not silence colors on an attended stderr, and a
/// redirected stderr must not colorize because stdout is a terminal.
pub fn stderr_color() -> bool {
    stream_color(std::io::stderr().is_terminal())
}

fn stream_color(tty: bool) -> bool {
    tty && std::env::var_os("NO_COLOR")
        .filter(|v| !v.is_empty())
        .is_none()
}

/// A `warning: ...` line for stderr; the label is yellow on a styled
/// terminal and plain otherwise.
pub fn warning(message: impl Display) -> String {
    label(
        "warning",
        message,
        stderr_color(),
        "warning".yellow().to_string(),
    )
}

/// An `error: ...` line for stderr; the label is red on a styled terminal
/// and plain otherwise.
pub fn error(message: impl Display) -> String {
    label("error", message, stderr_color(), "error".red().to_string())
}

fn label(name: &str, message: impl Display, color: bool, styled: String) -> String {
    format!(
        "{}: {message}",
        if color { styled } else { name.to_owned() }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_plain_without_color() {
        assert_eq!(
            label("warning", "boom", false, "warning".to_owned()),
            "warning: boom"
        );
        assert_eq!(
            label("error", "boom", false, "error".to_owned()),
            "error: boom"
        );
    }
    #[test]
    fn labels_are_colored_with_color() {
        assert_eq!(
            label("warning", "boom", true, "warning".yellow().to_string()),
            "\u{1b}[33mwarning\u{1b}[39m: boom"
        );
        assert_eq!(
            label("error", "boom", true, "error".red().to_string()),
            "\u{1b}[31merror\u{1b}[39m: boom"
        );
    }
}
