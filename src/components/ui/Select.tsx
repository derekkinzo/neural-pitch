// Select — native <select> styled for the dark theme.
//
// We deliberately wrap the native element rather than build a Listbox: the
// Phase 1.2 surface has fewer than ten options per dropdown, the keyboard
// model is exactly what users expect, and the screen-reader experience is
// "free". Phase 4 may swap this for a Combobox if the movable-do solfege
// formatter introduces typeahead requirements.
//
// Cross-references:
//   docs/design/DESIGN.md §6 (Settings drawer controls)

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

const BASE =
  "w-full rounded-md border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-slate-100 " +
  "shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400 " +
  "focus-visible:ring-offset-2 focus-visible:ring-offset-slate-950 disabled:opacity-50";

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
