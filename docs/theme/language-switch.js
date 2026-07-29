(() => {
    const current = document.documentElement.lang.startsWith("ko") ? "ko" : "en";
    const target = current === "ko" ? "en" : "ko";
    const segments = window.location.pathname.split("/");
    const languageIndex = segments.lastIndexOf(current);

    try {
        window.localStorage.setItem("yo-docs-language", current);
    } catch {
        // Language switching still works when storage is unavailable.
    }

    if (languageIndex === -1) {
        return;
    }

    segments[languageIndex] = target;

    const link = document.createElement("a");
    link.className = "yo-language-switch";
    link.href = `${segments.join("/")}${window.location.search}${window.location.hash}`;
    link.hreflang = target;
    link.lang = target;
    link.textContent = target === "ko" ? "한국어" : "English";
    link.title = target === "ko" ? "한국어 문서로 전환" : "Switch to English";
    link.setAttribute("aria-label", link.title);
    document.querySelector("#mdbook-menu-bar .right-buttons")?.prepend(link);
})();
