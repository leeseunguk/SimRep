/** Brand mark: an einzel lens in section (three electrode pairs) with the ion beam through it. */
export function Logo({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 32 32" aria-hidden>
      <rect x="3" y="5" width="7" height="6" rx="1.5" fill="currentColor" opacity="0.55" />
      <rect x="12.5" y="3" width="7" height="8" rx="1.5" fill="currentColor" />
      <rect x="22" y="5" width="7" height="6" rx="1.5" fill="currentColor" opacity="0.55" />
      <rect x="3" y="21" width="7" height="6" rx="1.5" fill="currentColor" opacity="0.55" />
      <rect x="12.5" y="21" width="7" height="8" rx="1.5" fill="currentColor" />
      <rect x="22" y="21" width="7" height="6" rx="1.5" fill="currentColor" opacity="0.55" />
      <path d="M1 16 C 9 16, 11 13.5, 16 13.5 S 23 16, 31 16" fill="none" stroke="var(--color-accent)" strokeWidth="2.25" strokeLinecap="round" />
      <path d="M1 16 C 9 16, 11 18.5, 16 18.5 S 23 16, 31 16" fill="none" stroke="var(--color-accent)" strokeWidth="2.25" strokeLinecap="round" opacity="0.6" />
    </svg>
  );
}
