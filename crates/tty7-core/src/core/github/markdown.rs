//! Making an issue's Markdown safe to hand to the text view.
//!
//! Issue bodies and comments are written by anyone on the internet. The text
//! view renders Markdown natively (no browser, no script), so there is nothing
//! to execute — but two things in it act on the reader's machine:
//!
//! - **Images load themselves.** An `![](…)` or `<img src>` is fetched the
//!   moment it is drawn, which tells whoever controls that URL that this
//!   issue was opened, from which IP, and when. Only images GitHub itself
//!   hosts (a screenshot pasted into an issue, see [`is_github_hosted`]) are
//!   drawn — reading the issue already told GitHub as much. Any other image
//!   becomes a plain link: the reader can still open it, deliberately, in
//!   the browser. (github.com proxies third-party images through its own
//!   servers for the same reason; tty7 has no proxy, so it does not load them.)
//! - **Links open whatever they name.** A click hands the URL to the OS, and
//!   `file:///…` or an app's custom scheme can launch programs. Only `http`,
//!   `https` and `mailto` targets survive; anything else is pointed at `#`.
//!
//! This is a rewrite of the *source text*, not a Markdown parser. It knows
//! enough structure to leave code alone (fenced blocks and inline code spans
//! are copied verbatim, since `![x](y)` inside backticks is text) and to find
//! the link and image forms Markdown and the HTML subset actually have.

/// Rewrite `src` as described in the module docs. `image_label` names an image
/// whose alt text is empty ("image"), in the reader's language.
pub fn sanitize(src: &str, image_label: &str) -> String {
    let mut out = String::with_capacity(src.len() + 16);
    let mut fence: Option<(char, usize)> = None;
    for line in src.split_inclusive('\n') {
        let trimmed = line.trim_start_matches(' ');
        let indent = line.len() - trimmed.len();
        let marker = fence_marker(trimmed).filter(|_| indent <= 3);
        match (fence, marker) {
            (None, Some(m)) => {
                fence = Some(m);
                out.push_str(line);
                continue;
            }
            (Some((ch, n)), Some((mch, mn))) if ch == mch && mn >= n && closes_fence(trimmed) => {
                fence = None;
                out.push_str(line);
                continue;
            }
            (Some(_), _) => {
                out.push_str(line);
                continue;
            }
            (None, None) => {}
        }
        out.push_str(&sanitize_line(line, image_label));
    }
    out
}

/// A fence opener/closer: three or more of one of `` ` `` / `~`.
fn fence_marker(line: &str) -> Option<(char, usize)> {
    let ch = line.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = line.chars().take_while(|c| *c == ch).count();
    (n >= 3).then_some((ch, n))
}

/// A closing fence carries nothing after its marker but whitespace.
fn closes_fence(line: &str) -> bool {
    let ch = line.chars().next().unwrap_or(' ');
    line.trim_start_matches(ch).trim().is_empty()
}

/// One line outside a fenced block: code spans kept, the rest rewritten.
fn sanitize_line(line: &str, image_label: &str) -> String {
    // A reference definition, `[id]: url`, is a link target of its own.
    if let Some(rewritten) = reference_definition(line) {
        return rewritten;
    }
    // A table row cannot be split across paragraphs without ending the table.
    let own_block = !line.trim_start().starts_with('|');
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while !rest.is_empty() {
        match rest.find('`') {
            Some(start) => {
                out.push_str(&sanitize_text(&rest[..start], image_label, own_block));
                let after = &rest[start..];
                let ticks = after.chars().take_while(|c| *c == '`').count();
                let closer = "`".repeat(ticks);
                match after[ticks..].find(&closer) {
                    Some(end) => {
                        let span = ticks + end + ticks;
                        out.push_str(&after[..span]);
                        rest = &after[span..];
                    }
                    // An unclosed run of backticks is literal text.
                    None => {
                        out.push_str(&after[..ticks]);
                        rest = &after[ticks..];
                    }
                }
            }
            None => {
                out.push_str(&sanitize_text(rest, image_label, own_block));
                break;
            }
        }
    }
    out
}

fn reference_definition(line: &str) -> Option<String> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 || !trimmed.starts_with('[') {
        return None;
    }
    let close = trimmed.find("]:")?;
    if trimmed[1..close].contains(']') {
        return None;
    }
    let head_len = line.len() - trimmed.len() + close + 2;
    let tail = &line[head_len..];
    let target_start = tail.len() - tail.trim_start().len();
    let target = tail[target_start..].split_whitespace().next()?;
    let bare = target.trim_start_matches('<').trim_end_matches('>');
    if is_safe_target(bare) {
        return None;
    }
    let from = head_len + target_start;
    Some(format!(
        "{}#{}",
        &line[..from],
        &line[from + target.len()..]
    ))
}

/// Plain text between code spans. `own_block` puts each image kept as an
/// image in a paragraph of its own: the text view draws an image that shares
/// a paragraph with text at line height, a screenshot shrunk to an icon.
fn sanitize_text(text: &str, image_label: &str, own_block: bool) -> String {
    let image = |alt: &str, url: &str| match own_block {
        true => format!("\n\n![{alt}]({url})\n\n"),
        false => format!("![{alt}]({url})"),
    };
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        // A GitHub-hosted `![alt](url)` stays an image.
        if rest.starts_with("![")
            && !escaped(bytes, i)
            && let Some((alt, url, len)) = inline_image(rest)
            && is_github_hosted(url)
        {
            out.push_str(&image(alt, url));
            i += len;
            continue;
        }
        // Any other `![alt](…)` / `![alt][ref]` → a link with the same target.
        if rest.starts_with("![") && !escaped(bytes, i) {
            out.push('[');
            if rest[2..].starts_with(']') {
                out.push_str(image_label);
            }
            i += 2;
            continue;
        }
        // `](target` — the target half of an inline link or image.
        if rest.starts_with("](") {
            out.push_str("](");
            i += 2;
            let target = &text[i..];
            let lead = target.len() - target.trim_start().len();
            let body = &target[lead..];
            let (url, len) = if let Some(inner) = body.strip_prefix('<') {
                let end = inner.find('>').unwrap_or(inner.len());
                (&inner[..end], end + 2)
            } else {
                let end = body
                    .find(|c: char| c.is_whitespace() || c == ')')
                    .unwrap_or(body.len());
                (&body[..end], end)
            };
            out.push_str(&target[..lead]);
            if is_safe_target(url) {
                out.push_str(&body[..len.min(body.len())]);
            } else {
                out.push('#');
            }
            i += lead + len.min(body.len());
            continue;
        }
        if rest.starts_with('<') {
            // `<img …>` → a link to what it would have loaded.
            if starts_with_tag(rest, "img") {
                let end = rest.find('>').map_or(rest.len(), |e| e + 1);
                let tag = &rest[..end];
                let src = attr(tag, "src").unwrap_or_default();
                let alt = attr(tag, "alt").filter(|a| !a.trim().is_empty());
                let label = alt.as_deref().unwrap_or(image_label);
                let label = label.replace(['[', ']'], "");
                if is_github_hosted(&src) {
                    out.push_str(&image(&label, &src.replace(' ', "%20")));
                } else if is_safe_target(&src) && !src.is_empty() {
                    out.push_str(&format!("[{label}]({})", src.replace(' ', "%20")));
                } else {
                    out.push_str(&format!("[{label}](#)"));
                }
                i += end;
                continue;
            }
            // `<scheme:…>` autolink with a scheme we do not open.
            if let Some(end) = rest.find('>') {
                let inner = &rest[1..end];
                if !inner.contains(char::is_whitespace)
                    && scheme(inner).is_some()
                    && !is_safe_target(inner)
                {
                    out.push_str("\\<");
                    i += 1;
                    continue;
                }
            }
            // Any other tag: neutralise `href`/`src` values it carries.
            if let Some(end) = tag_end(rest) {
                out.push_str(&rewrite_tag_urls(&rest[..end]));
                i += end;
                continue;
            }
        }
        let ch = rest.chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn escaped(bytes: &[u8], i: usize) -> bool {
    let mut n = 0;
    let mut j = i;
    while j > 0 && bytes[j - 1] == b'\\' {
        n += 1;
        j -= 1;
    }
    n % 2 == 1
}

fn starts_with_tag(s: &str, name: &str) -> bool {
    let Some(rest) = s.strip_prefix('<') else {
        return false;
    };
    rest.len() > name.len()
        && rest[..name.len()].eq_ignore_ascii_case(name)
        && rest[name.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_whitespace() || c == '>' || c == '/')
}

/// The length of an HTML tag starting at `s[0] == '<'`, when it is one.
fn tag_end(s: &str) -> Option<usize> {
    let rest = s.strip_prefix('<')?;
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    if !rest.chars().next()?.is_ascii_alphabetic() {
        return None;
    }
    // Stop at the next `<`: a stray `<` in prose is not a tag.
    let end = s.find('>')?;
    (!s[1..end].contains('<')).then_some(end + 1)
}

fn rewrite_tag_urls(tag: &str) -> String {
    let mut tag = tag.to_string();
    for name in ["href", "src", "srcset", "poster", "background"] {
        if let Some(value) = attr(&tag, name)
            && !is_safe_target(&value)
        {
            tag = tag.replacen(&value, "#", 1);
        }
    }
    tag
}

/// An attribute's value, quoted or bare. Case-insensitive on the name.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find(name) {
        let at = from + pos;
        from = at + name.len();
        let before_ok = lower[..at]
            .chars()
            .last()
            .is_some_and(|c| c.is_whitespace());
        let after = lower[from..].trim_start();
        if !before_ok || !after.starts_with('=') {
            continue;
        }
        let offset = tag.len() - tag[from..].trim_start().len() + 1;
        let value = tag[offset..].trim_start();
        return Some(match value.chars().next() {
            Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or("").to_string(),
            _ => value
                .split(|c: char| c.is_whitespace() || c == '>')
                .next()
                .unwrap_or("")
                .trim_end_matches('/')
                .to_string(),
        });
    }
    None
}

/// The scheme of `url`, when it has one: letters first, then letters, digits,
/// `+`, `-`, `.`, up to a `:` that comes before any `/`, `?` or `#`.
fn scheme(url: &str) -> Option<&str> {
    let colon = url.find(':')?;
    let head = &url[..colon];
    if head.is_empty()
        || url[..colon].contains(['/', '?', '#'])
        || !head.chars().next()?.is_ascii_alphabetic()
        || !head
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    Some(head)
}

/// `![alt](url)` or `![alt](url "title")` at the start of `s`: the alt text,
/// the URL, and how many bytes the whole form takes. `None` for anything
/// else, including the reference form `![alt][id]`.
fn inline_image(s: &str) -> Option<(&str, &str, usize)> {
    let rest = s.strip_prefix("![")?;
    let alt_end = rest.find(']')?;
    let alt = &rest[..alt_end];
    if alt.contains('[') {
        return None;
    }
    let target = rest[alt_end + 1..].strip_prefix('(')?;
    let close = target.find(')')?;
    let inside = target[..close].trim();
    let url = inside.split_whitespace().next()?;
    let url = url
        .strip_prefix('<')
        .and_then(|u| u.strip_suffix('>'))
        .unwrap_or(url);
    Some((alt, url, 2 + alt_end + 1 + 1 + close + 1))
}

/// Whether an image URL is one GitHub serves itself: attachments pasted into
/// an issue (`github.com/user-attachments/…`, a repository's `/assets/…`) and
/// anything under `githubusercontent.com` (older attachments, raw files,
/// avatars, GitHub's own image proxy). Only `https`.
pub fn is_github_hosted(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    // Userinfo or a port would make the "host" above something else.
    if host.contains(['@', ':']) {
        return false;
    }
    let host = host.to_ascii_lowercase();
    if host == "github.com" {
        let mut parts = path.trim_start_matches('/').split('/');
        return match (parts.next(), parts.next(), parts.next()) {
            (Some("user-attachments"), Some(_), _) => true,
            (Some(_), Some(_), Some("assets")) => true,
            _ => false,
        };
    }
    host.ends_with(".githubusercontent.com")
}

/// Whether a link target may be handed to the OS opener.
///
/// Relative targets and fragments carry no scheme and so cannot name a program;
/// they are kept. Whitespace and control characters are stripped first, the
/// way a browser does, so ` jav\tascript:` is judged as `javascript:`.
pub fn is_safe_target(url: &str) -> bool {
    let cleaned: String = url
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect();
    match scheme(&cleaned) {
        None => !cleaned.contains(':') || cleaned.starts_with(['/', '#', '?', '.']),
        Some(s) => matches!(s.to_ascii_lowercase().as_str(), "http" | "https" | "mailto"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(src: &str) -> String {
        sanitize(src, "image")
    }

    #[test]
    fn github_hosted_images_stay_images() {
        let pasted = "https://github.com/user-attachments/assets/352d14e0-d11c";
        // Each in a paragraph of its own, so it is drawn at its size.
        assert_eq!(
            s(&format!("![shot]({pasted})")),
            format!("\n\n![shot]({pasted})\n\n")
        );
        assert_eq!(
            s(&format!("a ![](<{pasted}> \"t\") b")),
            format!("a \n\n![]({pasted})\n\n b")
        );
        assert_eq!(
            s(
                r#"<img width="300" alt="Screenshot" src="https://user-images.githubusercontent.com/1/a.png">"#
            ),
            "\n\n![Screenshot](https://user-images.githubusercontent.com/1/a.png)\n\n"
        );
        assert_eq!(
            s(&format!(r#"<img src="{pasted}" />"#)),
            format!("\n\n![image]({pasted})\n\n")
        );
        // In a table row the image stays in its cell.
        assert_eq!(
            s(&format!("| ![x]({pasted}) |")),
            format!("| ![x]({pasted}) |")
        );
    }

    #[test]
    fn only_github_itself_counts_as_github_hosted() {
        for yes in [
            "https://github.com/user-attachments/assets/x",
            "https://github.com/owner/repo/assets/1/x",
            "https://private-user-images.githubusercontent.com/1/x.png?jwt=y",
            "https://camo.githubusercontent.com/abc",
            "https://RAW.githubusercontent.com/o/r/main/a.png",
        ] {
            assert!(is_github_hosted(yes), "{yes}");
        }
        for no in [
            "http://github.com/user-attachments/assets/x",
            "https://github.com/owner/repo",
            "https://github.com.evil.io/user-attachments/assets/x",
            "https://evil.io/x.githubusercontent.com/a.png",
            "https://githubusercontent.com.evil.io/a.png",
            "https://github.com@evil.io/user-attachments/assets/x",
            "https://x.io/a.png",
            "",
        ] {
            assert!(!is_github_hosted(no), "{no}");
        }
    }

    #[test]
    fn images_become_links_to_the_same_target() {
        assert_eq!(
            s("see ![shot](https://x.io/a.png) here"),
            "see [shot](https://x.io/a.png) here"
        );
        assert_eq!(s("![](https://x.io/a.png)"), "[image](https://x.io/a.png)");
        assert_eq!(s("![ref style][1]"), "[ref style][1]");
        // An escaped bang is text, and stays text.
        assert_eq!(s(r"\![not](x)"), r"\![not](x)");
    }

    #[test]
    fn html_images_become_links_too() {
        assert_eq!(
            s(r#"<img width="300" alt="Screenshot" src="https://x.io/u/a.png">"#),
            "[Screenshot](https://x.io/u/a.png)"
        );
        assert_eq!(
            s("<IMG SRC='https://x.io/b.png' />"),
            "[image](https://x.io/b.png)"
        );
        assert_eq!(s(r#"<img src="file:///etc/passwd">"#), "[image](#)");
    }

    #[test]
    fn links_to_anything_but_the_web_are_disarmed() {
        assert_eq!(s("[ok](https://github.com)"), "[ok](https://github.com)");
        assert_eq!(s("[mail](mailto:a@b.c)"), "[mail](mailto:a@b.c)");
        assert_eq!(s("[rel](docs/a.md)"), "[rel](docs/a.md)");
        assert_eq!(s("[frag](#heading)"), "[frag](#heading)");
        assert_eq!(s("[x](file:///Applications/Calc.app)"), "[x](#)");
        assert_eq!(s("[x](javascript:alert(1))"), "[x](#))");
        assert_eq!(s("[x](<vscode://open?x=1> \"t\")"), "[x](# \"t\")");
        assert_eq!(s("[x]( ssh://host )"), "[x]( # )");
        assert_eq!(s("<a href=\"file:///x\">x</a>"), "<a href=\"#\">x</a>");
        assert_eq!(
            s("<a href=\"https://ok.io\">x</a>"),
            "<a href=\"https://ok.io\">x</a>"
        );
        assert_eq!(s("<smb://share/x>"), "\\<smb://share/x>");
        assert_eq!(s("<https://ok.io>"), "<https://ok.io>");
        assert_eq!(s("[r]: file:///x \"t\"\n"), "[r]: # \"t\"\n");
        assert_eq!(s("[r]: https://ok.io\n"), "[r]: https://ok.io\n");
    }

    #[test]
    fn code_is_left_exactly_as_written() {
        let fenced = "```md\n![x](http://a/b.png)\n<img src=\"x\">\n```\n![y](http://c/d.png)\n";
        assert_eq!(
            s(fenced),
            "```md\n![x](http://a/b.png)\n<img src=\"x\">\n```\n[y](http://c/d.png)\n"
        );
        assert_eq!(
            s("use `![a](b)` for images, ![c](http://d)"),
            "use `![a](b)` for images, [c](http://d)"
        );
        assert_eq!(
            s("~~~~\n[x](file:///y)\n~~~~\n"),
            "~~~~\n[x](file:///y)\n~~~~\n"
        );
    }

    #[test]
    fn prose_with_angle_brackets_is_untouched() {
        assert_eq!(s("a < b and c > d"), "a < b and c > d");
        assert_eq!(s("Vec<String> is fine"), "Vec<String> is fine");
        assert_eq!(s("unicode ✓ → ok"), "unicode ✓ → ok");
    }

    #[test]
    fn safe_targets_are_judged_after_stripping_whitespace() {
        assert!(!is_safe_target(" jav\tascript:alert(1)"));
        assert!(!is_safe_target("FILE:///x"));
        assert!(is_safe_target("HTTPS://x.io"));
        assert!(is_safe_target("./a:b"));
        assert!(is_safe_target("/abs/path"));
    }
}
