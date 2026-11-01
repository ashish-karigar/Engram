import type { SVGProps } from "react";

type IconProps = SVGProps<SVGSVGElement> & { size?: number };

/** Line icons on a 24px grid, drawn with the current text colour. */
function Icon({ size = 16, children, ...rest }: IconProps & { children: React.ReactNode }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.7} strokeLinecap="round" strokeLinejoin="round" aria-hidden {...rest}>
      {children}
    </svg>
  );
}

export const PlusIcon = (p: IconProps) => <Icon {...p}><path d="M12 5v14M5 12h14" /></Icon>;
export const CloseIcon = (p: IconProps) => <Icon {...p}><path d="M6 6l12 12M18 6L6 18" /></Icon>;
export const InfoIcon = (p: IconProps) => <Icon {...p}><circle cx="12" cy="12" r="9" /><path d="M12 11v5M12 8h.01" /></Icon>;
export const FolderIcon = (p: IconProps) => <Icon {...p}><path d="M3 7.5A1.5 1.5 0 0 1 4.5 6H9l2 2h8.5A1.5 1.5 0 0 1 21 9.5v8a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z" /></Icon>;
export const SettingsIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M10.3 4.3a1.7 1.7 0 0 1 3.4 0 1.7 1.7 0 0 0 2.6 1 1.7 1.7 0 0 1 2.4 2.4 1.7 1.7 0 0 0 1 2.6 1.7 1.7 0 0 1 0 3.4 1.7 1.7 0 0 0-1 2.6 1.7 1.7 0 0 1-2.4 2.4 1.7 1.7 0 0 0-2.6 1 1.7 1.7 0 0 1-3.4 0 1.7 1.7 0 0 0-2.6-1 1.7 1.7 0 0 1-2.4-2.4 1.7 1.7 0 0 0-1-2.6 1.7 1.7 0 0 1 0-3.4 1.7 1.7 0 0 0 1-2.6 1.7 1.7 0 0 1 2.4-2.4 1.7 1.7 0 0 0 2.6-1z" />
    <circle cx="12" cy="12" r="3" />
  </Icon>
);
export const LockIcon = (p: IconProps) => <Icon {...p}><rect x="5" y="11" width="14" height="9" rx="2" /><path d="M8 11V8a4 4 0 0 1 8 0v3" /></Icon>;
export const GraphIcon = (p: IconProps) => <Icon {...p}><circle cx="6" cy="17" r="2.2" /><circle cx="12" cy="6.5" r="2.6" /><circle cx="18" cy="16" r="2.2" /><path d="M7.2 15.2l3.6-6.4M13.3 8.8l3.6 5.4M8.2 16.8h7.6" /></Icon>;
export const PlugIcon = (p: IconProps) => <Icon {...p}><path d="M9 3v5M15 3v5M7 8h10v3a5 5 0 0 1-10 0zM12 16v5" /></Icon>;

/** The Engram mark: a stored memory, a dot held inside a ring. One colour, the text's. */
export function Mark({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" aria-hidden>
      <circle cx="8" cy="8" r="6.2" fill="none" stroke="currentColor" strokeWidth="1.5" />
      <circle cx="8" cy="8" r="2.3" fill="currentColor" />
    </svg>
  );
}
