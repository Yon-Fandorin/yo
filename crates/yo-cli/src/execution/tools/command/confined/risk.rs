//! 실행이나 확장 없이 직접 명령 형태만 판별하는 workspace-v2 정책.

use std::{mem, path::Path};

pub(super) const REVISION: &str = "yo.command-risk/workspace-v2";

#[derive(Default, Debug)]
pub(super) struct Assessment {
    pub(super) approval: bool,
    pub(super) network: bool,
    pub(super) git_write: bool,
    pub(super) writes: Vec<String>,
    pub(super) git_directories: Vec<String>,
}

#[derive(Clone, Default, Debug)]
struct Word {
    text: String,
    expanded: bool,
    glob: bool,
    redirect: bool,
}

impl Word {
    fn literal(&self) -> bool {
        !self.expanded && !self.glob
    }
    fn command(&self) -> &str {
        if self.literal() {
            Path::new(&self.text)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
        } else {
            ""
        }
    }
}

pub(super) fn assess(command: &str) -> Assessment {
    let mut result = Assessment::default();
    let mut deletion_count = 0;
    for words in simple_commands(command) {
        for pair in words.windows(2) {
            if pair[0].redirect && pair[0].text.starts_with('>') && pair[1].literal() {
                result.writes.push(pair[1].text.clone());
            }
        }
        let mut ordinary = Vec::new();
        let mut skip_target = false;
        for word in words {
            if skip_target {
                skip_target = false;
                continue;
            }
            if word.redirect {
                skip_target = true;
                continue;
            }
            ordinary.push(word);
        }
        let words = ordinary;
        let start = words
            .iter()
            .position(|word| !assignment(word))
            .unwrap_or(words.len());
        let words = &words[start..];
        let Some(program) = words.first() else {
            continue;
        };
        let arguments = &words[1..];
        match program.command() {
            "rm" | "unlink" => {
                let mut options = true;
                for word in arguments {
                    if options && word.text == "--" {
                        options = false;
                        continue;
                    }
                    if options && word.text.starts_with('-') {
                        result.approval |= word.text == "--recursive"
                            || (!word.text.starts_with("--")
                                && word.text[1..].contains(['r', 'R']));
                        continue;
                    }
                    deletion_count += 1;
                    result.approval |= !word.literal();
                    if word.literal() {
                        result.writes.push(word.text.clone());
                    }
                }
            },
            "find" => {
                result.approval |= arguments
                    .iter()
                    .any(|word| word.literal() && word.text == "-delete")
                    || arguments.windows(2).any(|pair| {
                        matches!(pair[0].text.as_str(), "-exec" | "-execdir")
                            && matches!(pair[1].command(), "rm" | "unlink")
                    });
                if result.approval {
                    result.writes.extend(
                        arguments
                            .iter()
                            .take_while(|word| !word.text.starts_with('-'))
                            .filter(|word| word.literal())
                            .map(|word| word.text.clone()),
                    );
                }
            },
            "xargs" => {
                // 옵션 값도 포함해 직접 실행 위치의 삭제 명령을 보수적으로 식별한다.
                let mut index = 0;
                while let Some(word) = arguments.get(index) {
                    if word.text == "--" {
                        index += 1;
                        break;
                    }
                    if matches!(word.text.as_str(), "-I" | "-n" | "-L" | "-P" | "-s" | "-E") {
                        index += 2;
                    } else if word.text.starts_with('-') {
                        index += 1;
                    } else {
                        break;
                    }
                }
                result.approval |= arguments
                    .get(index)
                    .is_some_and(|word| matches!(word.command(), "rm" | "unlink"));
            },
            "git" => assess_git(arguments, &mut result),
            "curl" | "wget" | "ssh" | "scp" | "sftp" | "rsync" => result.network = true,
            _ => {},
        }
    }
    result.approval |= deletion_count > 1;
    result
}

fn assignment(word: &Word) -> bool {
    word.literal()
        && word.text.split_once('=').is_some_and(|(key, _)| {
            !key.is_empty()
                && key.chars().enumerate().all(|(index, c)| {
                    c == '_' || c.is_ascii_alphabetic() || (index > 0 && c.is_ascii_digit())
                })
        })
}

fn assess_git(words: &[Word], result: &mut Assessment) {
    let mut index = 0;
    while let Some(word) = words.get(index) {
        if !word.literal() {
            return;
        }
        if matches!(word.text.as_str(), "--help" | "-h" | "--version") {
            return;
        }
        if matches!(
            word.text.as_str(),
            "-C" | "--git-dir" | "--work-tree" | "-c"
        ) {
            let Some(value) = words.get(index + 1) else {
                return;
            };
            if !value.literal() {
                return;
            }
            if word.text != "-c" {
                result.git_directories.push(value.text.clone());
            }
            index += 2;
        } else if word.text.starts_with('-') {
            if let Some((flag, value)) = word.text.split_once('=')
                && matches!(flag, "--git-dir" | "--work-tree")
            {
                result.git_directories.push(value.to_owned());
            }
            index += 1;
        } else {
            break;
        }
    }
    let Some(operation) = words.get(index) else {
        return;
    };
    let args = &words[index + 1..];
    if operation.text == "clean" {
        assess_git_clean(args, result);
        return;
    }
    let mut separator = false;
    let mut ambiguous = false;
    let mut options = Vec::new();
    let mut args = args.iter();
    while let Some(word) = args.next() {
        if word.literal() && word.text == "--" {
            separator = true;
            break;
        }
        ambiguous |= !word.literal();
        options.push(word);
        if git_value_option(&operation.text, &word.text) {
            ambiguous |= args.next().is_none_or(|value| !value.literal());
        }
    }
    let has = |option: &str| {
        options
            .iter()
            .any(|word| word.literal() && word.text == option)
    };
    if !ambiguous && (has("--help") || has("-h") || operation.text == "help") {
        return;
    }
    let short = |flag: char| {
        options.iter().any(|word| {
            word.literal()
                && word.text.starts_with('-')
                && !word.text.starts_with("--")
                && git_short_options(&operation.text, &word.text)
                    .0
                    .contains(flag)
        })
    };
    result.network |= matches!(
        operation.text.as_str(),
        "fetch" | "pull" | "push" | "clone" | "ls-remote"
    );
    result.git_write |= matches!(
        operation.text.as_str(),
        "add"
            | "commit"
            | "reset"
            | "checkout"
            | "switch"
            | "restore"
            | "clean"
            | "merge"
            | "rebase"
            | "cherry-pick"
            | "revert"
            | "branch"
            | "tag"
            | "stash"
            | "update-ref"
            | "config"
            | "worktree"
            | "gc"
            | "init"
    );
    result.approval |= match operation.text.as_str() {
        "reset" => has("--hard"),
        "switch" => short('f') || has("--force") || has("--discard-changes"),
        "checkout" => {
            short('f')
                || has("--force")
                || has("--discard-changes")
                || separator
                || !(has("-b") || has("--orphan"))
        },
        "restore" => {
            effective_git_flag(
                &operation.text,
                &options,
                "--worktree",
                "--no-worktree",
                'W',
            ) || !effective_git_flag(&operation.text, &options, "--staged", "--no-staged", 'S')
        },
        _ => false,
    };
    result.approval |= ambiguous
        && matches!(
            operation.text.as_str(),
            "reset" | "switch" | "checkout" | "restore"
        );
}

fn git_value_option(operation: &str, option: &str) -> bool {
    if git_short_options(operation, option).1 {
        return true;
    }
    matches!(operation, "reset" | "checkout" | "restore")
        && matches!(
            option,
            "--pathspec-from-file" | "-U" | "--unified" | "--inter-hunk-context"
        )
        || matches!(operation, "checkout" | "switch" | "restore") && option == "--conflict"
        || operation == "restore" && matches!(option, "-s" | "--source")
        || operation == "checkout" && matches!(option, "-b" | "-B" | "--orphan")
        || operation == "switch"
            && matches!(
                option,
                "-c" | "-C" | "--create" | "--force-create" | "--orphan"
            )
}

// 짧은 option의 값이 시작되면 나머지 문자는 flag가 아니라 붙은 값이다.
fn git_short_options<'a>(operation: &str, option: &'a str) -> (&'a str, bool) {
    let Some(flags) = option
        .strip_prefix('-')
        .filter(|_| !option.starts_with("--"))
    else {
        return ("", false);
    };
    for (index, flag) in flags.char_indices() {
        let takes_value = matches!(operation, "reset" | "checkout" | "restore") && flag == 'U'
            || operation == "restore" && flag == 's'
            || operation == "checkout" && matches!(flag, 'b' | 'B')
            || operation == "switch" && matches!(flag, 'c' | 'C');
        if takes_value {
            return (&flags[..index], index + flag.len_utf8() == flags.len());
        }
    }
    (flags, false)
}

fn effective_git_flag(
    operation: &str,
    options: &[&Word],
    positive: &str,
    negative: &str,
    short: char,
) -> bool {
    options.iter().fold(false, |enabled, word| {
        if !word.literal() {
            enabled
        } else if word.text == negative {
            false
        } else if word.text == positive
            || word.text.starts_with('-')
                && !word.text.starts_with("--")
                && git_short_options(operation, &word.text).0.contains(short)
        {
            true
        } else {
            enabled
        }
    })
}

// clean의 exclude 값은 옵션처럼 보여도 소비하고, dry-run의 마지막 설정을 따른다.
fn assess_git_clean(args: &[Word], result: &mut Assessment) {
    let mut dry_run = false;
    let mut ambiguous = false;
    let mut index = 0;
    while let Some(word) = args.get(index) {
        if !word.literal() {
            ambiguous = true;
            break;
        }
        let option = word.text.as_str();
        if option == "--" {
            break;
        }
        match option {
            "--help" | "-h" => return,
            "--dry-run" => dry_run = true,
            "--no-dry-run" => dry_run = false,
            "--exclude" | "-e" => {
                if args.get(index + 1).is_none_or(|value| !value.literal()) {
                    ambiguous = true;
                    break;
                }
                index += 1;
            },
            "--force" | "--no-force" | "--quiet" | "--no-quiet" | "--interactive"
            | "--no-interactive" => {},
            _ if option.starts_with("--exclude=") => {},
            _ if option.starts_with("--") => {
                ambiguous = true;
                break;
            },
            _ if option.starts_with('-') => {
                let mut flags = option[1..].chars().peekable();
                while let Some(flag) = flags.next() {
                    match flag {
                        'n' => dry_run = true,
                        'e' => {
                            if flags.peek().is_none() {
                                if args.get(index + 1).is_none_or(|value| !value.literal()) {
                                    ambiguous = true;
                                } else {
                                    index += 1;
                                }
                            }
                            break;
                        },
                        'd' | 'f' | 'i' | 'q' | 'x' | 'X' => {},
                        _ => {
                            ambiguous = true;
                            break;
                        },
                    }
                }
                if ambiguous {
                    break;
                }
            },
            _ => {},
        }
        index += 1;
    }
    result.git_write = true;
    result.approval |= ambiguous || !dry_run;
}

fn simple_commands(input: &str) -> Vec<Vec<Word>> {
    let chars: Vec<char> = input.chars().collect();
    let mut result = Vec::new();
    let mut words = Vec::new();
    let mut word = Word::default();
    let mut started = false;
    let mut quote = None;
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if quote == Some('\'') {
            if c == '\'' {
                quote = None;
            } else {
                word.text.push(c);
            }
        } else if c == '\\' && index + 1 < chars.len() {
            index += 1;
            if chars[index] != '\n' {
                word.text.push(chars[index]);
                started = true;
            }
        } else if c == '"' {
            quote = if quote == Some('"') { None } else { Some('"') };
            started = true;
        } else if c == '\'' && quote.is_none() {
            quote = Some('\'');
            started = true;
        } else if c == '$' || c == '`' {
            word.expanded = true;
            word.text.push(c);
            started = true;
            // 중첩 인터프리터 본문은 별도 직접 명령으로 분해하지 않는다.
            if c == '`' || chars.get(index + 1) == Some(&'(') {
                let closing = if c == '`' { '`' } else { ')' };
                let mut depth = 1;
                if c == '$' {
                    index += 1;
                    word.text.push('(');
                }
                while index + 1 < chars.len() {
                    index += 1;
                    let nested = chars[index];
                    word.text.push(nested);
                    if closing == ')' && nested == '(' {
                        depth += 1;
                    }
                    if nested == closing {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
            }
        } else if quote.is_some() {
            word.text.push(c);
            started = true;
        } else if c == '#' && !started {
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            if !words.is_empty() {
                result.push(mem::take(&mut words));
            }
            continue;
        } else if matches!(c, ';' | '\n' | '&' | '|') {
            if started {
                words.push(mem::take(&mut word));
                started = false;
            }
            if !words.is_empty() {
                result.push(mem::take(&mut words));
            }
        } else if c.is_whitespace() {
            if started {
                words.push(mem::take(&mut word));
                started = false;
            }
        } else if c == '>' || c == '<' {
            if started {
                if !word.text.chars().all(|c| c.is_ascii_digit()) {
                    words.push(mem::take(&mut word));
                } else {
                    word = Word::default();
                }
                started = false;
            }
            let mut operator = c.to_string();
            if chars.get(index + 1).is_some_and(|c| matches!(c, '>' | '|')) {
                index += 1;
                operator.push(chars[index]);
            }
            words.push(Word {
                text: operator,
                redirect: true,
                ..Word::default()
            });
        } else {
            word.glob |= matches!(c, '*' | '?' | '[');
            word.text.push(c);
            started = true;
        }
        index += 1;
    }
    if started {
        words.push(word);
    }
    if !words.is_empty() {
        result.push(words);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::assess;

    // 최소 삭제/Git 표의 위험 명령은 경로 접두부·따옴표·복합 명령에서도 승인을 요구한다.
    #[test]
    fn recognized_risks_require_approval() {
        for command in [
            "rm -rf one",
            "rm --recursive one",
            "rm a b",
            "rm a; unlink b",
            "rm *.rs",
            "rm \"$file\"",
            "rm $(printf one)",
            "rm $((1+1))",
            "find . -delete",
            "find . -exec /bin/rm {} \\;",
            "find . -execdir unlink {} +",
            "printf x | xargs -0 /bin/rm",
            "git clean -fd",
            "git clean -f -- --help",
            "git clean -f -- -h",
            "git clean -f -- --dry-run",
            "git clean -f -- -n",
            "git clean -f -e --help",
            "git clean -f -e -h",
            "git clean -f -e -n",
            "git clean -f -e --dry-run",
            "git clean -f --exclude --help",
            "git clean -f --exclude=--help",
            "git clean -f -e--help",
            "git clean -fe-n",
            "git clean -n --no-dry-run -f",
            "git clean -n --unknown --help",
            "git clean -n -e $PATTERN",
            "git clean -n --exclude $PATTERN",
            "git clean -ne $PATTERN",
            "git restore -- --help",
            "git restore -- -h",
            "git restore -- --staged",
            "git restore -- -S",
            "git restore -sS file",
            "git restore -sfooS file",
            "git restore -s --staged file",
            "git restore --pathspec-from-file --help",
            "git restore --pathspec-from-file --staged",
            "git restore --staged --no-staged file",
            "git restore --staged --pathspec-from-file $PATHS",
            "git checkout --pathspec-from-file --help",
            "git checkout -b new -- file",
            "git -C . -c a=b reset --hard",
            "git --git-dir=.git checkout -f branch",
            "git --work-tree . switch --discard-changes branch",
            "git checkout -- file",
            "git checkout branch",
            "git restore file",
            "git restore -SW file",
        ] {
            assert!(assess(command).approval, "{command}");
        }
    }

    // 익숙하지 않은 실행 파일과 문자 데이터는 위험으로 추측하지 않으며 단일 literal 삭제를
    // 허용한다.
    #[test]
    fn ordinary_commands_and_literal_data_remain_automatic() {
        for command in [
            "rm -f -- one",
            "unlink 'one file'",
            "rm '*.rs'",
            "rm a\\*b",
            "rm one > output",
            "rm one 2> output",
            "rm one < input",
            "A=b /bin/rm one",
            "echo 'rm -rf x'",
            "echo ok # rm -rf .",
            "git add . && git commit -m 'rm -rf'",
            "git clean -nd",
            "git clean -n -- --help",
            "git clean --dry-run -- -n",
            "git clean -n -f -e --help",
            "git clean -n -f -e --dry-run",
            "git clean -ne--help",
            "git clean -f --no-dry-run -n",
            "git clean --no-dry-run --dry-run -f",
            "git clean -n -e '*.kept'",
            "git clean -n --exclude 'ignored pattern'",
            "git restore --staged -- --worktree",
            "git restore -S -- -W",
            "git restore --staged --pathspec-from-file --worktree",
            "git restore --staged --worktree --no-worktree file",
            "git clean --help",
            "git --help clean -fd",
            "git reset --hard --help",
            "git restore -SsHEAD file",
            "git restore --staged file",
            "git checkout -b new",
            "git switch branch",
            "unknown --something",
            "xargs echo rm",
            "echo ' > ' /outside",
            "python -c 'delete_everything()'",
        ] {
            assert!(!assess(command).approval, "{command}");
        }
    }

    // 외부 접근 요청은 위험 승인과 별도로 고정할 네트워크·쓰기 범위로 전달한다.
    #[test]
    fn scope_requests_are_separate_from_destructive_risk() {
        let assessment =
            assess("git -C ../other add .; echo ok > ../out; curl https://example.test");
        assert!(assessment.network);
        assert!(assessment.git_write);
        assert_eq!(assessment.git_directories, ["../other"]);
        assert_eq!(assessment.writes, ["../out"]);
    }
}
