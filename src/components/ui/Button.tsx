// Button — minimal accessible button primitive.
//
// Vendored copy of the shadcn-style Button primitive without
// slot/asChild composition. The surface is `<button {...props}>` with
// sensible defaults for focus ring, disabled state, and the dark theme.
//

import { forwardRef, type ButtonHTMLAttributes } from "react";

export type ButtonVariant = "primary" | "ghost";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
}

const BASE =
  "inline-flex items-center justify-center rounded-md font-medium transition-colors " +
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400 " +
  "focus-visible:ring-offset-2 focus-visible:ring-offset-slate-950 " +
  "disabled:pointer-events-none disabled:opacity-50";

const VARIANTS: Record<ButtonVariant, string> = {
  primary: "bg-cyan-500 text-slate-950 hover:bg-cyan-400 px-4 py-2 text-sm",
  ghost: "bg-transparent text-slate-200 hover:bg-slate-800 px-3 py-2 text-sm",
};

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "primary", className, type, ...rest },
  ref,
) {
  const cls = [BASE, VARIANTS[variant], className ?? ""].join(" ").trim();
  return <button ref={ref} type={type ?? "button"} className={cls} {...rest} />;
});
