import Link from "next/link";

export function Header() {
  return (
    <header className="sticky top-0 z-40 border-b border-[var(--color-border)] bg-[var(--color-background)]/85 backdrop-blur-md">
      <div className="mx-auto flex max-w-6xl items-center justify-between px-6 py-4">
        <Link href="/" className="flex items-center gap-2 group">
          <Logo />
          <span className="font-mono text-[15px] tracking-tight text-[var(--color-foreground)]">
            ratchet
          </span>
          <span className="chip chip-additive text-[10px] tracking-widest">
            v0.1
          </span>
        </Link>
        <nav className="flex items-center gap-5 text-sm text-[var(--color-muted)]">
          <Link
            href="/readiness"
            className="hover:text-[var(--color-foreground)] transition-colors"
          >
            Readiness
          </Link>
          <Link href="/diff" className="hover:text-[var(--color-foreground)] transition-colors">
            Diff
          </Link>
          <Link href="/rules" className="hover:text-[var(--color-foreground)] transition-colors">
            Rules
          </Link>
          <Link href="/skill.md" className="hover:text-[var(--color-foreground)] transition-colors">
            SKILL.md
          </Link>
          <a
            href="https://github.com/saicharanpogul/ratchet"
            target="_blank"
            rel="noreferrer"
            className="hover:text-[var(--color-foreground)] transition-colors"
          >
            GitHub ↗
          </a>
        </nav>
      </div>
    </header>
  );
}

function Logo() {
  // Stylized "R" built from two offset blocks, nodding at BEFORE/AFTER
  // in a diff. Purple for old, green for new — the same colors the
  // findings list uses for `old`/`new` values.
  return (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" aria-hidden>
      <rect
        x="3"
        y="3"
        width="9"
        height="18"
        rx="2"
        fill="var(--color-accent-purple)"
        opacity="0.85"
      />
      <rect
        x="12"
        y="7"
        width="9"
        height="14"
        rx="2"
        fill="var(--color-accent-green)"
        opacity="0.85"
      />
    </svg>
  );
}
