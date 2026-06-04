// App root — Phase 1.2 mounts the live tuner.
//
// Cross-references:
//   docs/design/DESIGN.md §7 (Phase 1.2 frontend design)

import { type ReactNode } from "react";
import { Tuner } from "@/components/Tuner";

export function App(): ReactNode {
  return <Tuner />;
}
