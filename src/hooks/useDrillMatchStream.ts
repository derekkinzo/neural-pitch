// useDrillMatchStream — Phase 4 match-update channel listener.
//
// Mirrors `usePitchStream`'s test-bridge wiring: the page-side hook
// registers itself on `__neuralPitchTestHooks.listeners.get("match-update")`
// while the drill is active and forwards each frame into the training
// store via `setLiveMatch(normaliseMatchUpdate(payload))`.
//
// Tear-down tolerates the receiver closing early — `registerTestListener`
// returns a no-op cleanup when the harness is not installed (production
// runs without it), and the test bridge's `pushMatchUpdate` itself
// short-circuits when the listener list is empty.
//

import { useEffect } from "react";
import { registerTestListener } from "@/lib/test-hooks";
import {
  normaliseMatchUpdate,
  useTrainingStore,
  type WireMatchUpdate,
} from "@/stores/trainingStore";
import type { MatchUpdate } from "@/types/training";

function asMatchUpdate(payload: unknown): MatchUpdate | null {
  if (payload === null || typeof payload !== "object") return null;
  return normaliseMatchUpdate(payload as WireMatchUpdate);
}

/**
 * Register a match-update listener while `active` is true. The listener
 * snapshots each frame into `trainingStore.liveMatch` and the
 * KaraokeRibbon repaints in its rAF loop. Passing `false` skips the
 * registration so the unused listener does not appear in the bridge's
 * fanout map (this keeps `pushMatchUpdate(...)` a no-op between
 * drills).
 */
export function useDrillMatchStream(active: boolean): void {
  useEffect(() => {
    if (!active) return undefined;
    const setLiveMatch = useTrainingStore.getState().setLiveMatch;
    const teardown = registerTestListener("match-update", (payload) => {
      const update = asMatchUpdate(payload);
      if (update === null) return;
      setLiveMatch(update);
    });
    return () => {
      teardown();
    };
  }, [active]);
}
