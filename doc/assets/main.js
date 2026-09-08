(() => {
  const root = document.documentElement;
  const toggle = document.querySelector(".theme-toggle");
  const themeMeta = document.querySelector('meta[name="theme-color"]');
  const header = document.querySelector(".site-header");
  const progress = document.querySelector(".scroll-progress");
  const pointerLight = document.querySelector(".pointer-light");
  const navLinks = [...document.querySelectorAll(".nav-links a")];
  const sections = navLinks
    .map((link) => document.querySelector(link.getAttribute("href")))
    .filter(Boolean);
  const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  const finePointer = window.matchMedia("(hover: hover) and (pointer: fine)");

  const syncTheme = () => {
    const isDark = root.dataset.theme !== "light";
    if (toggle) {
      toggle.setAttribute("aria-pressed", String(!isDark));
      toggle.title = isDark ? "切换到浅色主题" : "切换到深色主题";
    }
    if (themeMeta) themeMeta.content = isDark ? "#070811" : "#eff1f8";
  };

  toggle?.addEventListener("click", () => {
    root.dataset.theme = root.dataset.theme === "light" ? "dark" : "light";
    try {
      localStorage.setItem("vclogg2-site-theme", root.dataset.theme);
    } catch (_) {}
    syncTheme();
  });

  const revealGroups = [
    ".section-heading",
    ".feature-card",
    ".experience-copy",
    ".design-board",
    ".platform-grid article",
    ".start-card",
    ".trust-grid > div",
  ];
  const revealElements = [...document.querySelectorAll(revealGroups.join(","))];

  revealElements.forEach((element, index) => {
    element.dataset.reveal = "";
    element.style.setProperty("--reveal-delay", `${(index % 4) * 70}ms`);
  });

  if ("IntersectionObserver" in window && !reduceMotion.matches) {
    const revealObserver = new IntersectionObserver(
      (entries, observer) => {
        entries.forEach((entry) => {
          if (!entry.isIntersecting) return;
          entry.target.classList.add("is-visible");
          observer.unobserve(entry.target);
        });
      },
      { threshold: 0.13, rootMargin: "0px 0px -8%" },
    );
    revealElements.forEach((element) => revealObserver.observe(element));
  } else {
    revealElements.forEach((element) => element.classList.add("is-visible"));
  }

  const countElements = [...document.querySelectorAll("[data-count]")];
  const animateCount = (element) => {
    const target = Number(element.dataset.count || 0);
    const start = performance.now();
    const duration = 900;

    const tick = (now) => {
      const progressValue = Math.min((now - start) / duration, 1);
      const eased = 1 - Math.pow(1 - progressValue, 4);
      element.textContent = String(Math.round(target * eased));
      if (progressValue < 1) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  };

  if ("IntersectionObserver" in window) {
    const countObserver = new IntersectionObserver((entries, observer) => {
      entries.forEach((entry) => {
        if (!entry.isIntersecting) return;
        animateCount(entry.target);
        observer.unobserve(entry.target);
      });
    });
    countElements.forEach((element) => countObserver.observe(element));
  }

  let scrollTicking = false;
  const syncScrollState = () => {
    const scrollTop = window.scrollY;
    const scrollRange = document.documentElement.scrollHeight - window.innerHeight;
    const scrollRatio = scrollRange > 0 ? scrollTop / scrollRange : 0;

    header?.classList.toggle("scrolled", scrollTop > 20);
    progress?.style.setProperty("--scroll-progress", `${scrollRatio * 100}%`);

    const marker = scrollTop + window.innerHeight * 0.38;
    let current = "";
    for (const section of sections) {
      if (section.offsetTop <= marker) current = `#${section.id}`;
    }
    for (const link of navLinks) {
      link.classList.toggle("active", link.getAttribute("href") === current);
    }

    if (!reduceMotion.matches) {
      document.querySelectorAll("[data-parallax]").forEach((element) => {
        const speed = Number(element.dataset.parallax || 0);
        element.style.setProperty("--parallax-y", `${scrollTop * speed}px`);
      });
    }
    scrollTicking = false;
  };

  const requestScrollSync = () => {
    if (scrollTicking) return;
    scrollTicking = true;
    requestAnimationFrame(syncScrollState);
  };

  window.addEventListener("scroll", requestScrollSync, { passive: true });
  window.addEventListener("resize", requestScrollSync);

  if (finePointer.matches && !reduceMotion.matches) {
    window.addEventListener(
      "pointermove",
      (event) => {
        pointerLight?.style.setProperty("--pointer-x", `${event.clientX}px`);
        pointerLight?.style.setProperty("--pointer-y", `${event.clientY}px`);
      },
      { passive: true },
    );

    document.querySelectorAll("[data-tilt]").forEach((element) => {
      element.addEventListener("pointermove", (event) => {
        const rect = element.getBoundingClientRect();
        const x = (event.clientX - rect.left) / rect.width;
        const y = (event.clientY - rect.top) / rect.height;
        const rotateX = (0.5 - y) * 5;
        const rotateY = (x - 0.5) * 6;

        element.style.setProperty("--card-x", `${x * 100}%`);
        element.style.setProperty("--card-y", `${y * 100}%`);
        element.style.setProperty("--tilt-x", `${rotateX}deg`);
        element.style.setProperty("--tilt-y", `${rotateY}deg`);
      });

      element.addEventListener("pointerleave", () => {
        element.style.setProperty("--tilt-x", "0deg");
        element.style.setProperty("--tilt-y", "0deg");
      });
    });
  }

  syncTheme();
  syncScrollState();
})();
