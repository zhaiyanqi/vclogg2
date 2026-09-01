(() => {
  const root = document.documentElement;
  const toggle = document.querySelector(".theme-toggle");
  const themeMeta = document.querySelector('meta[name="theme-color"]');
  const header = document.querySelector(".site-header");
  const navLinks = [...document.querySelectorAll(".nav-links a")];
  const sections = navLinks
    .map((link) => document.querySelector(link.getAttribute("href")))
    .filter(Boolean);

  const syncTheme = () => {
    const isDark = root.dataset.theme !== "light";
    if (toggle) {
      toggle.setAttribute("aria-pressed", String(!isDark));
      toggle.title = isDark ? "切换到浅色主题" : "切换到深色主题";
    }
    if (themeMeta) themeMeta.content = isDark ? "#0b1020" : "#eeeef0";
  };

  toggle?.addEventListener("click", () => {
    root.dataset.theme = root.dataset.theme === "light" ? "dark" : "light";
    try {
      localStorage.setItem("vclogg2-site-theme", root.dataset.theme);
    } catch (_) {}
    syncTheme();
  });

  const syncScrollState = () => {
    header?.classList.toggle("scrolled", window.scrollY > 12);

    const marker = window.scrollY + window.innerHeight * 0.35;
    let current = "";
    for (const section of sections) {
      if (section.offsetTop <= marker) current = `#${section.id}`;
    }
    for (const link of navLinks) {
      link.classList.toggle("active", link.getAttribute("href") === current);
    }
  };

  window.addEventListener("scroll", syncScrollState, { passive: true });
  window.addEventListener("resize", syncScrollState);
  syncTheme();
  syncScrollState();
})();
