import Link from "next/link";
import { CodeBlock } from "../components/CodeBlock";

export default function Home() {
  return (
    <>
      <Hero />
      <HowItWorks />
      <RulesTeaser />
      <Install />
    </>
  );
}

function Hero() {
  return (
    <section className="mx-auto max-w-6xl px-6 pt-20 pb-24 md:pt-28 md:pb-32">
      <div className="flex flex-col items-start gap-6 max-w-3xl">
        <span className="chip">
          <span className="h-1.5 w-1.5 rounded-full bg-[var(--color-accent-green)]" />
          upgrade-safety for Solana programs
        </span>
        <h1 className="text-5xl md:text-6xl font-semibold tracking-tight leading-[1.05]">
          Catch breaking changes{" "}
          <span className="text-[var(--color-accent-purple)]">before</span>{" "}
          <span className="italic">solana program upgrade</span>{" "}
          lets them through.
        </h1>
        <p className="text-lg text-[var(--color-muted)] max-w-2xl leading-relaxed">
          ratchet diffs a candidate Anchor IDL against the deployed program (or
          a committed <code className="mono text-[var(--color-foreground)]">ratchet.lock</code> baseline)
          and classifies every change as additive, unsafe, or breaking — with
          the specific field, discriminator, or PDA seed that would corrupt
          existing state.
        </p>
        <div className="flex flex-wrap gap-3 pt-2">
          <Link
            href="/readiness"
            className="px-5 py-2.5 rounded-md bg-[var(--color-accent-purple)] hover:bg-[var(--color-accent-purple-dim)] text-white font-medium transition-colors"
          >
            Check readiness →
          </Link>
          <Link
            href="/diff"
            className="px-5 py-2.5 rounded-md border border-[var(--color-border-strong)] hover:border-[var(--color-muted)] text-[var(--color-foreground)] transition-colors"
          >
            Upgrade diff
          </Link>
          <a
            href="https://github.com/saicharanpogul/ratchet"
            target="_blank"
            rel="noreferrer"
            className="px-5 py-2.5 rounded-md border border-[var(--color-border-strong)] hover:border-[var(--color-muted)] text-[var(--color-foreground)] transition-colors"
          >
            View on GitHub
          </a>
        </div>
      </div>
    </section>
  );
}

function HowItWorks() {
  const steps = [
    {
      n: "01",
      title: "Snapshot a baseline",
      body: (
        <>
          <CodeBlock inline>{`ratchet lock --program <PID>`}</CodeBlock>{" "}
          writes a <code className="mono">ratchet.lock</code> — the current
          on-chain schema, committed to your repo.
        </>
      ),
    },
    {
      n: "02",
      title: "Diff on every PR",
      body: (
        <>
          <CodeBlock inline>{`ratchet check-upgrade --lock ratchet.lock --new <NEW_IDL>`}</CodeBlock>{" "}
          runs 16 rules and exits non-zero on anything that would silently
          corrupt data.
        </>
      ),
    },
    {
      n: "03",
      title: "Ship with confidence",
      body: (
        <>
          Intentional renames? Migration declared? Pass{" "}
          <CodeBlock inline>{`--unsafe allow-rename`}</CodeBlock>{" "}
          or{" "}
          <CodeBlock inline>{`--migrated-account Vault`}</CodeBlock>.
          Every acknowledgement is visible in the report.
        </>
      ),
    },
  ];
  return (
    <section className="mx-auto max-w-6xl px-6 py-16 border-t border-[var(--color-border)]">
      <SectionLabel>How it works</SectionLabel>
      <div className="mt-10 grid gap-6 md:grid-cols-3">
        {steps.map((s) => (
          <div
            key={s.n}
            className="group rounded-lg border border-[var(--color-border)] bg-[var(--color-background-subtle)] p-6 hover:border-[var(--color-border-strong)] transition-colors"
          >
            <div className="mono text-xs text-[var(--color-dim)] tracking-widest">
              {s.n}
            </div>
            <h3 className="mt-2 text-lg font-medium text-[var(--color-foreground)]">
              {s.title}
            </h3>
            <div className="mt-3 text-[15px] leading-relaxed text-[var(--color-muted)]">
              {s.body}
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}

function RulesTeaser() {
  const highlights = [
    {
      id: "R006",
      name: "account-discriminator-change",
      caption:
        "Catches struct renames before every existing account on-chain fails AccountDiscriminatorMismatch.",
    },
    {
      id: "R013",
      name: "pda-seed-change",
      caption:
        "Notices when the PDA seeds for an account input changed — every derived address is now at a different pubkey.",
    },
    {
      id: "R005",
      name: "account-field-append",
      caption:
        "Flags appends that would need a realloc. Auto-demoted when Anchor's realloc = ... constraint is in source.",
    },
  ];
  return (
    <section className="mx-auto max-w-6xl px-6 py-16 border-t border-[var(--color-border)]">
      <div className="flex items-baseline justify-between flex-wrap gap-4">
        <SectionLabel>Rules that fire</SectionLabel>
        <Link
          href="/rules"
          className="text-sm text-[var(--color-muted)] hover:text-[var(--color-foreground)] transition-colors"
        >
          See all 16 →
        </Link>
      </div>
      <div className="mt-8 grid gap-4 md:grid-cols-3">
        {highlights.map((r) => (
          <div
            key={r.id}
            className="rounded-lg border border-[var(--color-border)] p-5 bg-[var(--color-background-subtle)]"
          >
            <div className="flex items-center gap-2">
              <span className="chip chip-breaking mono">{r.id}</span>
              <code className="mono text-sm text-[var(--color-foreground)]">
                {r.name}
              </code>
            </div>
            <p className="mt-3 text-sm text-[var(--color-muted)] leading-relaxed">
              {r.caption}
            </p>
          </div>
        ))}
      </div>
    </section>
  );
}

function Install() {
  return (
    <section className="mx-auto max-w-6xl px-6 py-16 border-t border-[var(--color-border)]">
      <SectionLabel>Install</SectionLabel>
      <div className="mt-8 grid gap-4 md:grid-cols-2">
        <CodeBlock>{`cargo install solana-ratchet-cli`}</CodeBlock>
        <CodeBlock>{`# GitHub Action
- uses: saicharanpogul/ratchet@main
  with:
    new: target/idl/my_program.json
    lock: ratchet.lock`}</CodeBlock>
      </div>
    </section>
  );
}

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="mono text-xs text-[var(--color-dim)] tracking-widest uppercase">
      {children}
    </div>
  );
}
