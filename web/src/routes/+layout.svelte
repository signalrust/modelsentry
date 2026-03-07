<script lang="ts">
  import '../app.css';
  import { page } from '$app/state';
  import { onMount } from 'svelte';

  let { children } = $props();

  const THEMES = ['black', 'light', 'navy'] as const;
  type Theme = typeof THEMES[number];

  let theme: Theme = $state('black');
  let drawerOpen = $state(false);

  function setTheme(t: Theme) {
    theme = t;
    document.documentElement.setAttribute('data-theme', t);
    try { localStorage.setItem('ms-theme', t); } catch {}
  }

  onMount(() => {
    let saved = 'black' as Theme;
    try {
      const s = localStorage.getItem('ms-theme') as Theme | null;
      if (s && THEMES.includes(s)) saved = s;
    } catch {}
    setTheme(saved);
  });

  let path = $derived(page.url.pathname);

  function navClass(href: string) {
    if (href === '/') return path === '/' ? 'nav-item active' : 'nav-item';
    return path.startsWith(href) ? 'nav-item active' : 'nav-item';
  }

  function closeDrawer() { drawerOpen = false; }
</script>

<!-- Fixed header -->
<header class="app-header">
  <div class="header-left">
    <!-- Hamburger (mobile only) -->
    <button
      class="hamburger"
      class:open={drawerOpen}
      aria-label="Toggle menu"
      onclick={() => (drawerOpen = !drawerOpen)}
    >
      <span></span><span></span><span></span>
    </button>

    <a class="logo" href="/">
      Model<span class="logo-accent">Sentry</span>
    </a>

    <span class="live-badge">
      <span class="live-dot"></span>
      LIVE
    </span>
  </div>

  <div class="header-right">
    <!-- Theme toggle -->
    <div class="theme-toggle">
      {#each THEMES as t}
        <button
          class="theme-btn"
          class:active={theme === t}
          onclick={() => setTheme(t)}
          title={t}
        >{t}</button>
      {/each}
    </div>
  </div>
</header>

<!-- Sidebar overlay (mobile) -->
<div
  class="sidebar-overlay"
  class:open={drawerOpen}
  role="presentation"
  onclick={closeDrawer}
></div>

<!-- Mobile drawer -->
<nav class="app-sidebar-mobile" class:open={drawerOpen} aria-label="Mobile navigation">
  <div class="sidebar-nav">
    <div class="sidebar-section-label">Navigation</div>
    <a class={navClass('/')} href="/" onclick={closeDrawer}>
      <span class="nav-icon">⬛</span> Dashboard
    </a>
    <a class={navClass('/probes')} href="/probes" onclick={closeDrawer}>
      <span class="nav-icon">◎</span> Probes
    </a>
  </div>
</nav>

<!-- Desktop sidebar -->
<nav class="app-sidebar" aria-label="Main navigation">
  <div class="sidebar-nav">
    <div class="sidebar-section-label">Navigation</div>
    <a class={navClass('/')} href="/">
      <span class="nav-icon">⬛</span> Dashboard
    </a>
    <a class={navClass('/probes')} href="/probes">
      <span class="nav-icon">◎</span> Probes
    </a>
  </div>
</nav>

<!-- Main content -->
<main class="app-main">
  {@render children()}
</main>

<!-- Mobile bottom nav -->
<nav class="bottom-nav" aria-label="Bottom navigation">
  <a href="/" class={path === '/' ? 'active' : ''}>
    <span class="bottom-nav-icon">⬛</span>
    Dashboard
  </a>
  <a href="/probes" class={path.startsWith('/probes') ? 'active' : ''}>
    <span class="bottom-nav-icon">◎</span>
    Probes
  </a>
</nav>

