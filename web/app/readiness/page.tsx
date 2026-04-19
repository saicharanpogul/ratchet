"use client";

import { useCallback, useRef, useState } from "react";
import {
  checkReadiness,
  type Finding,
  type Report,
  type Severity,
} from "../../lib/ratchet";

export default function ReadinessPage() {
  const [idl, setIdl] = useState("");
  const [report, setReport] = useState<Report | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);

  const run = useCallback(async () => {
    setRunning(true);
    setReport(null);
    setError(null);
    try {
      const r = await checkReadiness(idl);
      setReport(r);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setRunning(false);
    }
  }, [idl]);

  const clear = () => {
    setIdl("");
    setReport(null);
    setError(null);
  };

  return (
    <section className="mx-auto max-w-6xl px-6 py-12">
      <div className="flex items-end justify-between flex-wrap gap-4">
        <div>
          <h1 className="text-4xl font-semibold tracking-tight">Readiness</h1>
          <p className="mt-2 text-[var(--color-muted)] max-w-2xl">
            Drop one Anchor IDL. ratchet runs the <code className="mono">P001–P006</code>{" "}
            design lints — version field, reserved padding, explicit discriminators,
            name collisions, unsignered writes — and tells you whether the shape
            is ready for mainnet or still has future-upgrade landmines.
          </p>
          <p className="mt-2 text-[var(--color-muted)] max-w-2xl">
            Use this before your first deploy.{" "}
            <a href="/diff" className="text-[var(--color-accent-purple)] underline">
              Already deployed? Use /diff to compare old vs new instead.
            </a>
          </p>
        </div>
        <div className="flex gap-2">
          <button
            onClick={clear}
            className="px-4 py-2 rounded-md border border-[var(--color-border)] hover:border-[var(--color-border-strong)] text-sm text-[var(--color-muted)] hover:text-[var(--color-foreground)] transition-colors"
          >
            Clear
          </button>
          <button
            onClick={run}
            disabled={running || !idl.trim()}
            className="px-5 py-2 rounded-md bg-[var(--color-accent-purple)] hover:bg-[var(--color-accent-purple-dim)] disabled:opacity-40 disabled:hover:bg-[var(--color-accent-purple)] text-white text-sm font-medium transition-colors"
          >
            {running ? "Running…" : "Run readiness"}
          </button>
        </div>
      </div>

      <div className="mt-8">
        <IdlDropzone value={idl} onChange={setIdl} />
      </div>

      {error && <ErrorCard message={error} />}
      {report && <ResultView report={report} />}
    </section>
  );
}

function IdlDropzone({
  value,
  onChange,
}: {
  value: string;
  onChange: (v: string) => void;
}) {
  const fileRef = useRef<HTMLInputElement | null>(null);
  const [dragging, setDragging] = useState(false);

  const onDrop = async (e: React.DragEvent) => {
    e.preventDefault();
    setDragging(false);
    const f = e.dataTransfer.files?.[0];
    if (f) onChange(await f.text());
  };
  const onPick = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const f = e.target.files?.[0];
    if (f) onChange(await f.text());
  };

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between text-sm">
        <span className="mono text-xs tracking-widest uppercase text-[var(--color-dim)]">
          Candidate IDL
        </span>
        <button
          onClick={() => fileRef.current?.click()}
          className="text-xs text-[var(--color-muted)] hover:text-[var(--color-foreground)] transition-colors"
        >
          upload file…
        </button>
        <input
          ref={fileRef}
          type="file"
          accept=".json,application/json"
          className="hidden"
          onChange={onPick}
        />
      </div>
      <div
        onDragOver={(e) => {
          e.preventDefault();
          setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={onDrop}
        className={`relative rounded-lg border ${
          dragging
            ? "border-[var(--color-border-strong)]"
            : "border-[var(--color-border)]"
        } bg-[var(--color-background-subtle)] transition-colors`}
        style={{
          boxShadow: dragging
            ? "inset 0 0 0 1px var(--color-accent-purple)"
            : undefined,
        }}
      >
        <div
          className="absolute top-0 left-0 h-1 rounded-t-lg"
          style={{
            width: "100%",
            background: "var(--color-accent-purple)",
            opacity: 0.6,
          }}
        />
        <textarea
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder="Paste an Anchor IDL JSON here, or drag the file in."
          className="mono w-full h-96 resize-none p-4 pt-5 bg-transparent outline-none text-sm text-[var(--color-foreground)] placeholder:text-[var(--color-dim)]"
          spellCheck={false}
        />
      </div>
    </div>
  );
}

function ErrorCard({ message }: { message: string }) {
  return (
    <div className="mt-8 rounded-lg border border-[var(--color-border)] p-5 bg-[var(--color-background-subtle)]">
      <div className="mono text-xs text-[var(--color-dim)] uppercase tracking-widest">
        Error
      </div>
      <div className="mt-2 text-sm text-[var(--color-foreground)] mono whitespace-pre-wrap">
        {message}
      </div>
    </div>
  );
}

function ResultView({ report }: { report: Report }) {
  const verdict = verdictOf(report);
  return (
    <div className="mt-10 space-y-6">
      <VerdictBanner verdict={verdict} report={report} />
      {report.findings.length === 0 ? (
        <div className="rounded-lg border border-[var(--color-border)] p-8 text-center text-[var(--color-muted)]">
          No readiness findings. The IDL looks mainnet-shaped against the six
          P-rules.
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          {report.findings.map((f, i) => (
            <FindingCard key={i} f={f} />
          ))}
        </div>
      )}
    </div>
  );
}

function verdictOf(r: Report): Severity | "safe" {
  const ranks: Record<Severity, number> = {
    additive: 0,
    unsafe: 1,
    breaking: 2,
  };
  let max = -1;
  let best: Severity = "additive";
  for (const f of r.findings) {
    const rank = ranks[f.severity];
    if (rank > max) {
      max = rank;
      best = f.severity;
    }
  }
  return max <= 0 ? "safe" : best;
}

function VerdictBanner({
  verdict,
  report,
}: {
  verdict: Severity | "safe";
  report: Report;
}) {
  const tone = {
    safe: {
      cls: "border-[var(--color-accent-green)]/40 bg-[color-mix(in_oklch,var(--color-accent-green)_8%,transparent)]",
      label: "READY",
      color: "var(--color-accent-green)",
      sub: "The IDL passes every P-rule with no outstanding concerns.",
    },
    additive: {
      cls: "border-[var(--color-accent-green)]/40 bg-[color-mix(in_oklch,var(--color-accent-green)_8%,transparent)]",
      label: "READY",
      color: "var(--color-accent-green)",
      sub: "Only informational findings — safe to deploy.",
    },
    unsafe: {
      cls: "border-[var(--color-unsafe)]/40 bg-[color-mix(in_oklch,var(--color-unsafe)_10%,transparent)]",
      label: "CONCERNS",
      color: "var(--color-unsafe)",
      sub: "Has future-upgrade landmines. Review each finding and either fix or acknowledge.",
    },
    breaking: {
      cls: "border-[var(--color-breaking)]/40 bg-[color-mix(in_oklch,var(--color-breaking)_12%,transparent)]",
      label: "BLOCKING",
      color: "var(--color-breaking)",
      sub: "Issues that will cause problems on mainnet. Fix before deploying.",
    },
  }[verdict];
  const counts = countBySeverity(report);
  return (
    <div
      className={`rounded-lg border p-5 flex items-center justify-between gap-4 flex-wrap ${tone.cls}`}
    >
      <div className="flex items-center gap-4">
        <span
          className="mono text-2xl font-semibold tracking-wide"
          style={{ color: tone.color }}
        >
          {tone.label}
        </span>
        <div className="flex flex-col text-sm">
          <span className="text-[var(--color-muted)]">
            {report.findings.length} finding
            {report.findings.length === 1 ? "" : "s"}
          </span>
          <span className="text-xs text-[var(--color-dim)]">{tone.sub}</span>
        </div>
      </div>
      <div className="flex gap-2 text-xs mono">
        {counts.breaking > 0 && (
          <span className="chip chip-breaking">{counts.breaking} breaking</span>
        )}
        {counts.unsafe > 0 && (
          <span className="chip chip-unsafe">{counts.unsafe} unsafe</span>
        )}
        {counts.additive > 0 && (
          <span className="chip chip-additive">{counts.additive} additive</span>
        )}
      </div>
    </div>
  );
}

function countBySeverity(r: Report) {
  const c = { additive: 0, unsafe: 0, breaking: 0 };
  for (const f of r.findings) c[f.severity] += 1;
  return c;
}

function FindingCard({ f }: { f: Finding }) {
  const sevCls = {
    breaking: "chip chip-breaking",
    unsafe: "chip chip-unsafe",
    additive: "chip chip-additive",
  }[f.severity];
  return (
    <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-background-subtle)] p-5">
      <div className="flex flex-wrap items-center gap-2 mb-3">
        <span className={`${sevCls} mono`}>{f.severity.toUpperCase()}</span>
        <code className="mono text-xs text-[var(--color-accent-purple)]">
          {f.rule_id}
        </code>
        <code className="mono text-sm text-[var(--color-foreground)]">
          {f.rule_name}
        </code>
        <span className="text-[var(--color-dim)]">·</span>
        <code className="mono text-xs text-[var(--color-muted)]">
          {f.path.join(" / ")}
        </code>
      </div>
      <p className="text-sm text-[var(--color-foreground)] leading-relaxed">
        {f.message}
      </p>
      {f.suggestion && (
        <p className="mt-3 text-xs text-[var(--color-muted)] leading-relaxed">
          <span className="text-[var(--color-dim)]">hint:</span> {f.suggestion}
        </p>
      )}
      {f.allow_flag && (
        <p className="mt-2 text-xs text-[var(--color-muted)]">
          acknowledge with{" "}
          <code className="mono text-[var(--color-foreground)]">
            --unsafe {f.allow_flag}
          </code>
        </p>
      )}
    </div>
  );
}
