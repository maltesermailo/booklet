//! Tags: the `#tag` words written in a note.
//!
//! A tag is `#` followed immediately by a letter — which is what separates it
//! from a `# Heading` (space after the `#`) — and it must start a word, so the
//! `#anchor` in a URL and the `#` of a colour literal are not tags.

/// The tags in `text`, in the order written, each without its `#`. A tag
/// repeated in the same note appears once.
pub fn tags_in(text: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    let mut fenced = false;

    for line in text.lines() {
        // A tag inside a code block is a shell comment or a preprocessor
        // directive, not something the note is about.
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }

        for tag in tags_in_line(line) {
            if !tags.contains(&tag) {
                tags.push(tag);
            }
        }
    }

    tags
}

fn tags_in_line(line: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut at_word_start = true;

    for (index, character) in line.char_indices() {
        if character == '#' && at_word_start {
            let rest = &line[index + 1..];
            let word: String = rest.chars().take_while(is_tag_char).collect();

            // `# Heading` and a bare `#` stop here: a tag's first character must
            // be a letter.
            if word.chars().next().is_some_and(char::is_alphabetic) {
                tags.push(word);
            }
        }

        at_word_start = character.is_whitespace();
    }

    tags
}

fn is_tag_char(character: &char) -> bool {
    character.is_alphanumeric() || *character == '-' || *character == '_' || *character == '/'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_tags_in_the_order_written_without_repeats() {
        let text = "# KGDB setup\n\nA note about #kernel and #debugging.\nMore #kernel talk.\n";

        assert_eq!(tags_in(text), ["kernel", "debugging"]);
    }

    #[test]
    fn a_heading_is_not_a_tag() {
        assert!(tags_in("# Heading\n## Deeper\n\nPlain text.\n").is_empty());
    }

    #[test]
    fn a_hash_mid_word_is_not_a_tag() {
        // Anchors, colour literals, and C# — none of them start a word.
        let text = "See [docs](https://example.com/page#anchor), colour #3C5240, C#.\n";

        assert!(tags_in(text).is_empty());
    }

    #[test]
    fn tags_may_hold_dashes_slashes_and_digits() {
        assert_eq!(tags_in("#pixel-7 #os/linux #trace32"), ["pixel-7", "os/linux", "trace32"]);
    }

    #[test]
    fn code_blocks_hold_no_tags() {
        let text = "Real #tag here.\n\n```sh\n# not a tag\n#alsonot\n```\n\nAnd #another.\n";

        assert_eq!(tags_in(text), ["tag", "another"]);
    }

    #[test]
    fn punctuation_ends_a_tag() {
        assert_eq!(tags_in("Ends at #kernel, and at #uart."), ["kernel", "uart"]);
    }
}
