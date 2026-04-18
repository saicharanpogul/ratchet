"use client";

import { useCallback, useRef, useState } from "react";

type Severity = "additive" | "unsafe" | "breaking";

interface Finding {
  rule_id: string;
  rule_name: string;
  severity: Severity;
  path: string[];
  message: string;
  old?: string;
  new?: string;
  suggestion?: string;
  allow_flag?: string;
}

interface Report {
  findings: Finding[];
}

interface CheckResult {
  ok: boolean;
  exit_code?: number;
  report?: Report;
  error?: string;
  stderr?: string;
}

export default function DiffPage() {
  const [oldJson, setOldJson] = useState("");
  const [newJson, setNewJson] = useState("");
  const [result, setResult] = useState<CheckResult | null>(null);
  const [running, setRunning] = useState(false);

  const runDiff = useCallback(async () => {
    setRunning(true);
    setResult(null);
    try {
      const res = await fetch("/api/check", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ old: oldJson, new: newJson }),
      });
      const body: CheckResult = await res.json();
      setResult(body);
    } catch (e) {
      setResult({ ok: false, error: String(e) });
    } finally {
      setRunning(false);
    }
  }, [oldJson, newJson]);

  const clear = () => {
    setOldJson("");
    setNewJson("");
    setResult(null);
  };

  return (
    <section className="mx-auto max-w-6xl px-6 py-12">
      <div className="flex items-end justify-between flex-wrap gap-4">
        <div>
          <h1 className="text-4xl font-semibold tracking-tight">Diff</h1>
          <p className="mt-2 text-[var(--color-muted)] max-w-2xl">
            Paste or drop two Anchor IDL JSON files. ratchet runs all 16 rules
            server-side and reports every change with the exact path, old/new
            values, and the allow-flag (if any) that could demote it.
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
            onClick={runDiff}
            disabled={running || !oldJson.trim() || !newJson.trim()}
            className="px-5 py-2 rounded-md bg-[var(--color-accent-purple)] hover:bg-[var(--color-accent-purple-dim)] disabled:opacity-40 disabled:hover:bg-[var(--color-accent-purple)] text-white text-sm font-medium transition-colors"
          >
            {running ? "Running…" : "Run check-upgrade"}
          </button>
        </div>
      </div>

      <div className="mt-8 grid gap-4 md:grid-cols-2">
        <IdlDropzone
          label="Old IDL (deployed)"
          accent="var(--color-accent-purple)"
          value={oldJson}
          onChange={setOldJson}
        />
        <IdlDropzone
          label="New IDL (candidate)"
          accent="var(--color-accent-green)"
          value={newJson}
          onChange={setNewJson}
        />
      </div>

      {result && <ResultView result={result} />}
    </section>
  );
}

function IdlDropzone({
  label,
  accent,
  value,
  onChange,
}: {
  label: string;
  accent: string;
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
          {label}
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
        style={{ boxShadow: dragging ? `inset 0 0 0 1px ${accent}` : undefined }}
      >
        <div
          className="absolute top-0 left-0 h-1 rounded-t-lg"
          style={{ width: "100%", background: accent, opacity: 0.6 }}
        />
        <textarea
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder='Paste IDL JSON here, or drag the file in / use the upload link above.'
          className="mono w-full h-72 resize-none p-4 pt-5 bg-transparent outline-none text-sm text-[var(--color-foreground)] placeholder:text-[var(--color-dim)]"
          spellCheck={false}
        />
      </div>
    </div>
  );
}

function ResultView({ result }: { result: CheckResult }) {
  if (!result.ok) {
    return (
      <div className="mt-8 rounded-lg border border-[var(--color-border)] p-5 bg-[var(--color-background-subtle)]">
        <div className="mono text-xs text-[var(--color-dim)] uppercase tracking-widest">
          Error
        </div>
        <div className="mt-2 text-sm text-[var(--color-foreground)]">
          {result.error ?? "Unknown error"}
        </div>
        {result.stderr && (
          <pre className="mono mt-3 text-xs text-[var(--color-muted)] whitespace-pre-wrap">
            {result.stderr}
          </pre>
        )}
      </div>
    );
  }

  const report = result.report ?? { findings: [] };
  const verdict = verdictOf(report);

  return (
    <div className="mt-10 space-y-6">
      <VerdictBanner verdict={verdict} report={report} exitCode={result.exit_code ?? 0} />
      {report.findings.length === 0 ? (
        <div className="rounded-lg border border-[var(--color-border)] p-8 text-center text-[var(--color-muted)]">
          No findings. The surfaces match exactly.
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
  exitCode,
}: {
  verdict: Severity | "safe";
  report: Report;
  exitCode: number;
}) {
  const tone = {
    safe: {
      cls: "border-[var(--color-accent-green)]/40 bg-[color-mix(in_oklch,var(--color-accent-green)_8%,transparent)]",
      label: "SAFE",
      color: "var(--color-accent-green)",
    },
    additive: {
      cls: "border-[var(--color-accent-green)]/40 bg-[color-mix(in_oklch,var(--color-accent-green)_8%,transparent)]",
      label: "ADDITIVE",
      color: "var(--color-accent-green)",
    },
    unsafe: {
      cls: "border-[var(--color-unsafe)]/40 bg-[color-mix(in_oklch,var(--color-unsafe)_10%,transparent)]",
      label: "UNSAFE",
      color: "var(--color-unsafe)",
    },
    breaking: {
      cls: "border-[var(--color-breaking)]/40 bg-[color-mix(in_oklch,var(--color-breaking)_12%,transparent)]",
      label: "BREAKING",
      color: "var(--color-breaking)",
    },
  }[verdict];
  const counts = countBySeverity(report);
  return (
    <div className={`rounded-lg border p-5 flex items-center justify-between gap-4 flex-wrap ${tone.cls}`}>
      <div className="flex items-center gap-4">
        <span
          className="mono text-2xl font-semibold tracking-wide"
          style={{ color: tone.color }}
        >
          {tone.label}
        </span>
        <span className="text-sm text-[var(--color-muted)]">
          {report.findings.length} finding{report.findings.length === 1 ? "" : "s"} ·
          exit <code className="mono">{exitCode}</code>
        </span>
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
        <code className="mono text-xs text-[var(--color-accent-purple)]">{f.rule_id}</code>
        <code className="mono text-sm text-[var(--color-foreground)]">{f.rule_name}</code>
        <span className="text-[var(--color-dim)]">·</span>
        <code className="mono text-xs text-[var(--color-muted)]">
          {f.path.join(" / ")}
        </code>
      </div>
      <p className="text-sm text-[var(--color-foreground)] leading-relaxed">
        {f.message}
      </p>
      {(f.old || f.new) && (
        <div className="mono mt-4 rounded-md border border-[var(--color-border)] overflow-hidden">
          {f.old && (
            <div className="px-3 py-2 bg-[color-mix(in_oklch,var(--color-accent-purple)_6%,transparent)] text-sm flex gap-3">
              <span className="text-[var(--color-accent-purple)]">-</span>
              <span className="text-[var(--color-foreground)] whitespace-pre-wrap">
                {f.old}
              </span>
            </div>
          )}
          {f.new && (
            <div className="px-3 py-2 bg-[color-mix(in_oklch,var(--color-accent-green)_6%,transparent)] border-t border-[var(--color-border)] text-sm flex gap-3">
              <span className="text-[var(--color-accent-green)]">+</span>
              <span className="text-[var(--color-foreground)] whitespace-pre-wrap">
                {f.new}
              </span>
            </div>
          )}
        </div>
      )}
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
