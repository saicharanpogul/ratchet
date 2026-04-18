import type { Metadata } from "next";
import { RULES } from "../../lib/rules";

export const metadata: Metadata = {
  title: "Rules · ratchet",
  description: "The 16 upgrade-safety rules ratchet enforces.",
};

const SEV_CLASS: Record<string, string> = {
  BREAKING: "chip chip-breaking",
  UNSAFE: "chip chip-unsafe",
  ADDITIVE: "chip chip-additive",
  MIXED: "chip",
};

export default function RulesPage() {
  return (
    <section className="mx-auto max-w-6xl px-6 py-16">
      <h1 className="text-4xl font-semibold tracking-tight">Rule catalog</h1>
      <p className="mt-3 text-[var(--color-muted)] max-w-2xl">
        16 upgrade-safety rules grouped by what they protect. Each has a stable
        ID that never changes once published; severities and allow flags are
        tuned conservatively — when in doubt ratchet flags.
      </p>

      <div className="mt-10 border border-[var(--color-border)] rounded-lg overflow-hidden">
        <div className="grid grid-cols-[64px_1fr_140px_1fr] px-5 py-3 bg-[var(--color-background-subtle)] border-b border-[var(--color-border)] text-xs uppercase tracking-widest text-[var(--color-dim)] mono">
          <div>ID</div>
          <div>Name</div>
          <div>Severity</div>
          <div>Allow flag</div>
        </div>
        {RULES.map((r) => (
          <div
            key={r.id}
            className="grid grid-cols-[64px_1fr_140px_1fr] px-5 py-4 border-b border-[var(--color-border)] last:border-b-0 text-sm items-start"
          >
            <div className="mono text-[var(--color-accent-purple)]">{r.id}</div>
            <div>
              <code className="mono text-[var(--color-foreground)] text-[13.5px]">
                {r.name}
              </code>
              <div className="text-[var(--color-muted)] mt-1 leading-relaxed">
                {r.description}
              </div>
            </div>
            <div>
              <span className={SEV_CLASS[r.severity] ?? "chip"}>{r.severity}</span>
            </div>
            <div className="mono text-xs text-[var(--color-muted)] leading-relaxed">
              {r.allow ?? <span className="text-[var(--color-dim)]">—</span>}
            </div>
          </div>
        ))}
      </div>

      <div className="mt-10 text-sm text-[var(--color-muted)] leading-relaxed space-y-3">
        <p>
          Pass an allow flag with <code className="mono">--unsafe &lt;flag&gt;</code> (e.g.{" "}
          <code className="mono">--unsafe allow-rename</code>).
        </p>
        <p>
          <code className="mono text-[var(--color-foreground)]">--migrated-account &lt;Name&gt;</code>{" "}
          demotes R003, R004, and R005 for that account — use when a{" "}
          <code className="mono">Migration&lt;From, To&gt;</code> or custom migration instruction
          exists.
        </p>
        <p>
          <code className="mono text-[var(--color-foreground)]">--realloc-account &lt;Name&gt;</code>{" "}
          demotes R005. Auto-populated from source when{" "}
          <code className="mono">--new-source</code> is provided and the field carries{" "}
          <code className="mono">#[account(realloc = ...)]</code>.
        </p>
      </div>
    </section>
  );
}
