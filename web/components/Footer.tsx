import Link from "next/link";

export function Footer() {
  return (
    <footer className="border-t border-[var(--color-border)] py-8 mt-20">
      <div className="mx-auto max-w-6xl px-6 flex flex-col md:flex-row items-start md:items-center justify-between gap-4 text-sm text-[var(--color-muted)]">
        <div className="flex items-center gap-3">
          <span className="font-mono">ratchet</span>
          <span className="text-[var(--color-dim)]">·</span>
          <span>Apache-2.0</span>
        </div>
        <div className="flex flex-wrap items-center gap-x-6 gap-y-2">
          <a
            href="https://github.com/saicharanpogul/ratchet"
            target="_blank"
            rel="noreferrer"
            className="hover:text-[var(--color-foreground)] transition-colors"
          >
            GitHub
          </a>
          <a
            href="https://crates.io/crates/solana-ratchet-cli"
            target="_blank"
            rel="noreferrer"
            className="hover:text-[var(--color-foreground)] transition-colors"
          >
            crates.io
          </a>
          <Link
            href="/skill.md"
            className="hover:text-[var(--color-foreground)] transition-colors"
          >
            SKILL.md
          </Link>
          <a
            href="https://github.com/saicharanpogul/ratchet/blob/main/CHANGELOG.md"
            target="_blank"
            rel="noreferrer"
            className="hover:text-[var(--color-foreground)] transition-colors"
          >
            Changelog
          </a>
        </div>
      </div>
    </footer>
  );
}
