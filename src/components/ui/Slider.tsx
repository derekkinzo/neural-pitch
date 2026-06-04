// Slider — native <input type="range"> styled for the dark theme.
//
// Native sliders are accessible by default (arrow-key fine adjustment, Home/
// End extremes, screen-reader value-now). The shadcn package's Slider primitive
// wraps Radix Slider for visual richness; for Phase 1.2 the native control is
// the right balance of low surface area + correct semantics.
//
// Cross-references:
//   docs/design/DESIGN.md §6 (Smoothing window slider)

import { forwardRef, type ChangeEvent, type InputHTMLAttributes } from "react";

export interface SliderProps extends Omit<
  InputHTMLAttributes<HTMLInputElement>,
  "onChange" | "value" | "type"
> {
  value: number;
  min: number;
  max: number;
  step?: number;
  onValueChange: (value: number) => void;
}

const BASE =
  "w-full cursor-pointer accent-cyan-400 focus-visible:outline-none " +
  "focus-visible:ring-2 focus-visible:ring-cyan-400 focus-visible:ring-offset-2 " +
  "focus-visible:ring-offset-slate-950";

export const Slider = forwardRef<HTMLInputElement, SliderProps>(function Slider(
  { value, min, max, step = 1, onValueChange, className, ...rest },
  ref,
) {
  const handleChange = (e: ChangeEvent<HTMLInputElement>): void => {
    const n = Number(e.currentTarget.value);
    if (Number.isFinite(n)) onValueChange(n);
  };
  const cls = [BASE, className ?? ""].join(" ").trim();
  return (
    <input
      ref={ref}
      type="range"
      min={min}
      max={max}
      step={step}
      value={value}
      onChange={handleChange}
      className={cls}
      {...rest}
    />
  );
});
