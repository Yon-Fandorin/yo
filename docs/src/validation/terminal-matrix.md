# Terminal environment matrix

Real PTY output is the authority for terminal behavior. HTML fixtures help
diagnosis and parity review but do not replace these checks.

## Linux environment checks

Local tmux, both presentation modes:

```bash
cargo test -p yo-cli --test terminal_matrix local_tmux_ \
  -- --ignored --nocapture --test-threads=1
```

SSH and tmux inside SSH, both presentation modes:

```bash
cargo test -p yo-cli --test terminal_matrix ssh:: \
  -- --ignored --nocapture --test-threads=1
```

The SSH tests start an isolated localhost `sshd`, generate temporary keys, and
remove their fixture directory. They require compatible local `ssh`, `sshd`,
`ssh-keygen`, Codex, and, for the nested cases, tmux.

## Current verification boundary

The executable matrix covers Inline and Fullscreen in local tmux, SSH, and tmux
inside SSH on Linux. macOS compilation is checked when a macOS host is
available; macOS environment behavior remains unverified until run on a real
host.

Contract: [Rendering validation authority](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.validation-matrix.md)
