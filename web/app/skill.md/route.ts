import { readFile } from "node:fs/promises";
import { join } from "node:path";

/**
 * GET /skill.md
 *
 * Serves the repo-root SKILL.md as text/markdown so agents (and humans
 * pasting `domain.com/skill.md` into anything) get the canonical skill
 * definition. Same source of truth as github.com/…/SKILL.md — we read
 * it at request time rather than copying into /public so the two never
 * drift.
 */
export async function GET() {
  const path = join(process.cwd(), "..", "SKILL.md");
  let body: string;
  try {
    body = await readFile(path, "utf8");
  } catch (e) {
    return new Response(
      `SKILL.md not found at ${path}: ${String(e)}`,
      { status: 500, headers: { "content-type": "text/plain" } },
    );
  }
  return new Response(body, {
    status: 200,
    headers: {
      "content-type": "text/markdown; charset=utf-8",
      "cache-control": "public, max-age=60",
    },
  });
}
