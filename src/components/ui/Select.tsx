// Select — native <select> styled for the dark theme.
//
// We deliberately wrap the native element rather than build a Listbox: the
// surface has fewer than ten options per dropdown, the keyboard
// model is exactly what users expect, and the screen-reader experience is
// "free". A Combobox is a possible swap if the movable-do solfege
// formatter ever introduces typeahead requirements.
//

import {
  forwardRef,
  type ChangeEvent,
  type ReactElement,
  type ReactNode,
  type Ref,
  type SelectHTMLAttributes,
} from "react";

export interface SelectProps<T extends string | number> extends Omit<
  SelectHTMLAttributes<HTMLSelectElement>,
  "onChange" | "value"
> {
  value: T;
  onValueChange: (value: T) => void;
  children: ReactNode;
  /** Forces numeric values to round-trip via Number() rather than string. */
  numeric?: boolean;
}

// `appearance-none` removes the platform's default <select> chrome (notably
// webkit2gtk's opaque white inner background), so the dark `bg-slate-900`
// actually applies. The chevron is restored via an inline SVG background so
// the control still reads as a dropdown. Without `appearance-none` the
// selected option text renders white-on-white inside the Tauri webview.
const BASE =
  "w-full appearance-none rounded-md border border-slate-700 bg-slate-900 px-3 py-2 pr-9 " +
  "text-sm text-slate-100 shadow-sm focus-visible:outline-none focus-visible:ring-2 " +
  "focus-visible:ring-cyan-400 focus-visible:ring-offset-2 focus-visible:ring-offset-slate-950 " +
  "disabled:opacity-50 bg-no-repeat bg-[length:1rem_1rem] bg-[right_0.5rem_center] " +
  "bg-[url('data:image/svg+xml;utf8,<svg xmlns=%22http://www.w3.org/2000/svg%22 fill=%22none%22 " +
  "viewBox=%220 0 20 20%22><path stroke=%22%23cbd5e1%22 stroke-width=%221.5%22 stroke-linecap=%22round%22 " +
  "stroke-linejoin=%22round%22 d=%22m6 8 4 4 4-4%22/></svg>')]";

function SelectInner<T extends string | number>(
  { value, onValueChange, children, className, numeric, ...rest }: SelectProps<T>,
  ref: Ref<HTMLSelectElement>,
): ReactElement {
  const handleChange = (e: ChangeEvent<HTMLSelectElement>): void => {
    const raw = e.currentTarget.value;
    if (numeric === true) {
      const n = Number(raw);
      if (Number.isFinite(n)) onValueChange(n as T);
    } else {
      onValueChange(raw as T);
    }
  };
  const cls = [BASE, className ?? ""].join(" ").trim();
  return (
    <select ref={ref} value={String(value)} onChange={handleChange} className={cls} {...rest}>
      {children}
    </select>
  );
}

export const Select = forwardRef(SelectInner) as <T extends string | number>(
  props: SelectProps<T> & { ref?: Ref<HTMLSelectElement> },
) => ReactElement;
