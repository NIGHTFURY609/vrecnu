// @vrecnu/ui's Button is styled for a light page; this window is dark. `cn` uses tailwind-merge,
// so these classes override the variant's.

/** Focus ring visible on slate-950 (the Button's default ring is dark-on-dark here). */
const FOCUS = "focus-visible:ring-slate-300 focus-visible:ring-offset-slate-950";
export const PRIMARY = `bg-slate-100 text-slate-900 hover:bg-white ${FOCUS}`;
export const SECONDARY = `border-slate-600 text-slate-100 hover:bg-slate-800 ${FOCUS}`;
export const DANGER = FOCUS;
export const FIELD =
  "rounded border border-slate-700 bg-slate-900 px-2 py-1.5 text-sm text-slate-100 disabled:text-slate-500";
