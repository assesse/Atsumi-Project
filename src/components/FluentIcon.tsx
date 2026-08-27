type FluentIconProps = {
  glyph: string;
  label?: string;
};

export function FluentIcon({ glyph, label }: FluentIconProps) {
  const escapedCodepoint = /^\\u[0-9a-f]{4}$/i.test(glyph)
    ? String.fromCharCode(Number.parseInt(glyph.slice(2), 16))
    : glyph;
  return (
    <span className="fluent" aria-hidden={label ? undefined : true} aria-label={label}>
      {escapedCodepoint}
    </span>
  );
}
