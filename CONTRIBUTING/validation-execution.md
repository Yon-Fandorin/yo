# Full workspace test execution

The current Codex Linux runner's workspace sandbox denies Unix socket binds
with `EPERM` in the full workspace test suite. When the full suite is required,
request unrestricted execution before the first run. Do not repeat a known
sandboxed failure as a permission probe.

If unrestricted execution is unavailable, run viable focused checks and report
the full suite as unverified. Keep platform gaps visible. Diagnose any failure
from an unrestricted run on its own merits; this permission rule makes no
test-pass claim or concurrency choice. Revisit it if the runner's Unix-socket
policy changes.
