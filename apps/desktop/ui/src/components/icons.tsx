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

/** Brain outline, facing right, with the stem below. */
const BRAIN = "M4 15 2 10 6 4 13 1.5 21 2 28 6 30.5 12 27 18 19 19.5 13 18 9 20.5 6 19z";
const STEM = "M14 18.3 15.5 25 17.5 19.4";
/** Inner links, as [x1, y1, x2, y2]. Small sizes draw only the first few. */
const LINKS = [
  [10, 8, 18, 7.5], [18, 7.5, 24, 11], [10, 8, 15, 13], [18, 7.5, 15, 13], [15, 13, 24, 11], [9, 14, 15, 13],
  [4, 15, 13, 18], [6, 4, 10, 8], [13, 1.5, 10, 8], [13, 1.5, 18, 7.5], [21, 2, 18, 7.5], [28, 6, 24, 11],
  [30.5, 12, 24, 11], [27, 18, 24, 11], [19, 19.5, 24, 11], [19, 19.5, 15, 13], [13, 18, 15, 13], [2, 10, 10, 8],
  [2, 10, 9, 14], [10, 8, 9, 14], [4, 15, 9, 14], [9, 14, 13, 18],
];
const NODES = [[10, 8], [18, 7.5], [24, 11], [15, 13], [9, 14]];

/**
 * The MemoryOS mark: a brain drawn as a wireframe of linked nodes, like the memory graph. One
 * colour, the text's. Below 20px only the main links are drawn, so it stays legible.
 */
export function Mark({ size = 16 }: { size?: number }) {
  const small = size < 20;
  const links = small ? LINKS.slice(0, 7) : LINKS;
  return (
    <svg width={size} height={size} viewBox="0 -3 32 32" fill="none" stroke="currentColor" strokeLinejoin="round" strokeLinecap="round" aria-hidden>
      <path d={BRAIN} strokeWidth={small ? 2.2 : 1.6} />
      <path d={STEM} strokeWidth={small ? 2.2 : 1.6} />
      <g strokeWidth={small ? 1.4 : 0.9} opacity={small ? 0.8 : 0.7}>
        {links.map(([x1, y1, x2, y2], i) => <line key={i} x1={x1} y1={y1} x2={x2} y2={y2} />)}
      </g>
      <g fill="currentColor" stroke="none">
        {NODES.map(([cx, cy], i) => <circle key={i} cx={cx} cy={cy} r={small ? 2 : 1.5} />)}
      </g>
    </svg>
  );
}
