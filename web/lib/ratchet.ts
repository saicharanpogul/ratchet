/**
 * Thin wrapper around the compiled `ratchet-wasm` module. Initialises
 * the WASM binary the first time it's called and caches the handle so
 * subsequent calls don't re-fetch.
 *
 * The returned `check` function is sync once the module is loaded —
 * the actual rule engine runs inside the wasm instance on the same
 * thread, no workers involved. Browsers will block the event loop for
 * the (sub-millisecond) duration of a check; worth measuring if the
 * rule count ever grows.
 */

import init, { check_upgrade, version } from "./ratchet-wasm/solana_ratchet_wasm.js";

export type Severity = "additive" | "unsafe" | "breaking";

export interface Finding {
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

export interface Report {
  findings: Finding[];
}

let ready: Promise<void> | null = null;

function ensureReady(): Promise<void> {
  if (ready) return ready;
  const p: Promise<void> = init().then(
    () => undefined,
    (err: unknown) => {
      ready = null;
      throw err;
    },
  );
  ready = p;
  return p;
}

export async function ratchetVersion(): Promise<string> {
  await ensureReady();
  return version();
}

/**
 * Diff two Anchor IDL JSON strings and return the Report. Throws when
 * either input fails to parse as Anchor IDL or the normalizer rejects
 * it — the thrown value's `.message` carries the detail.
 */
export async function checkUpgrade(
  oldIdl: string,
  newIdl: string,
): Promise<Report> {
  await ensureReady();
  const json = check_upgrade(oldIdl, newIdl);
  return JSON.parse(json) as Report;
}
