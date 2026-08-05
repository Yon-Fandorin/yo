//! Closed registry of tracked ContextBuild contract artifacts.

pub(crate) struct ContextManifestRegistration {
    pub(crate) request: &'static str,
    pub(crate) context: &'static str,
    pub(crate) manifest: &'static str,
}

pub(crate) const REGISTRATIONS: &[ContextManifestRegistration] = &[
    ContextManifestRegistration {
        request: "tools/methexis/examples/context-contract/direct-request.json",
        context: "tools/methexis/examples/context-contract/context.md",
        manifest: "tools/methexis/examples/context-contract/manifest.json",
    },
    ContextManifestRegistration {
        request: "tools/methexis/examples/context-contract/stable-leaf-request.json",
        context: "tools/methexis/examples/context-contract/stable-leaf-context.md",
        manifest: "tools/methexis/examples/context-contract/stable-leaf-manifest.json",
    },
];

pub(crate) fn manifest_paths() -> impl Iterator<Item = &'static str> {
    REGISTRATIONS.iter().map(|item| item.manifest)
}
