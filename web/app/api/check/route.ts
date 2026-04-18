import { NextRequest, NextResponse } from "next/server";
import { spawn } from "node:child_process";
import { writeFile, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

/**
 * POST /api/check
 *
 * Body: { old: string /* IDL JSON *\/, new: string /* IDL JSON *\/ }
 *
 * Shells out to the locally-installed `ratchet` CLI, parses its `--json`
 * output, and returns it to the client. In production the binary must be
 * available on PATH inside the container — the Dockerfile for this app
 * bakes it in. See web/README.md for deployment notes.
 *
 * Intentional rate-cap: requests are bounded by 256 KiB per IDL so a
 * stray paste of a gigantic payload can't DoS the dyno.
 */

const MAX_IDL_BYTES = 256 * 1024;

export async function POST(req: NextRequest) {
  let body: { old?: unknown; new?: unknown };
  try {
    body = await req.json();
  } catch {
    return NextResponse.json(
      { ok: false, error: "request body must be JSON" },
      { status: 400 },
    );
  }

  const oldJson = typeof body.old === "string" ? body.old : "";
  const newJson = typeof body.new === "string" ? body.new : "";
  if (!oldJson.trim() || !newJson.trim()) {
    return NextResponse.json(
      { ok: false, error: "both `old` and `new` IDL JSON strings are required" },
      { status: 400 },
    );
  }
  if (oldJson.length > MAX_IDL_BYTES || newJson.length > MAX_IDL_BYTES) {
    return NextResponse.json(
      { ok: false, error: `each IDL must be under ${MAX_IDL_BYTES} bytes` },
      { status: 413 },
    );
  }
  for (const [name, s] of Object.entries({ old: oldJson, new: newJson })) {
    try {
      JSON.parse(s);
    } catch (e) {
      return NextResponse.json(
        { ok: false, error: `${name} is not valid JSON: ${String(e)}` },
        { status: 400 },
      );
    }
  }

  const dir = await mkdtemp(join(tmpdir(), "ratchet-web-"));
  const oldPath = join(dir, "old.json");
  const newPath = join(dir, "new.json");
  try {
    await writeFile(oldPath, oldJson, "utf8");
    await writeFile(newPath, newJson, "utf8");

    const bin = process.env.RATCHET_BIN || "ratchet";
    const out = await runCli(bin, [
      "--json",
      "check-upgrade",
      "--old",
      oldPath,
      "--new",
      newPath,
    ]);

    if (out.exitCode === 3 || out.stderr.includes("ratchet: ")) {
      return NextResponse.json({
        ok: false,
        exit_code: out.exitCode,
        error: "ratchet reported a CLI error (malformed IDL?)",
        stderr: out.stderr,
      });
    }

    let report: unknown = null;
    try {
      report = JSON.parse(out.stdout);
    } catch {
      return NextResponse.json({
        ok: false,
        exit_code: out.exitCode,
        error: "could not parse ratchet's JSON output",
        stderr: out.stderr,
      });
    }

    return NextResponse.json({
      ok: true,
      exit_code: out.exitCode,
      report,
    });
  } catch (e) {
    return NextResponse.json({
      ok: false,
      error: String(e),
    });
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

function runCli(
  bin: string,
  args: string[],
): Promise<{ exitCode: number; stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    const p = spawn(bin, args, { stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    p.stdout.on("data", (b) => (stdout += b.toString()));
    p.stderr.on("data", (b) => (stderr += b.toString()));
    p.on("error", (err) => {
      reject(new Error(`failed to spawn ${bin}: ${err.message}`));
    });
    p.on("close", (code) => {
      resolve({ exitCode: code ?? -1, stdout, stderr });
    });
  });
}
