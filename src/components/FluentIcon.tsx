type FluentIconProps = {
  glyph: string;
  label?: string;
  className?: string;
};

export function FluentIcon({ glyph, label, className }: FluentIconProps) {
  const escapedCodepoint = /^\\u[0-9a-f]{4}$/i.test(glyph)
    ? String.fromCharCode(Number.parseInt(glyph.slice(2), 16))
    : glyph;
  return (
    <span className={`fluent${className ? ` ${className}` : ""}`} aria-hidden={label ? undefined : true} aria-label={label}>
      {escapedCodepoint}
    </span>
  );
}
