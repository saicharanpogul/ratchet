export function CodeBlock({
  children,
  inline = false,
}: {
  children: React.ReactNode;
  inline?: boolean;
}) {
  if (inline) {
    return (
      <code className="mono text-[0.85em] px-1.5 py-0.5 rounded border border-[var(--color-border)] bg-[var(--color-background-subtle)] text-[var(--color-foreground)]">
        {children}
      </code>
    );
  }
  return (
    <pre className="mono text-sm leading-relaxed rounded-lg border border-[var(--color-border)] bg-[var(--color-background-subtle)] p-4 overflow-x-auto text-[var(--color-foreground)]">
      <code>{children}</code>
    </pre>
  );
}
