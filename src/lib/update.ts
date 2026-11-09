// Whether a newer Omega Star is out, shared by the sidebar's update card and Settings: checked when the app
// opens, every hour after, and whenever you ask.
import { call, errorMessage } from "../api/client";
import { createStore, useStore } from "./store";

type UpdateCheck = {
  checking: boolean;
  /** The newer version, once a check found one. A failed check keeps what the last one found. */
  version: string | null;
  /** Why the last check failed, or null when it worked. */
  error: string | null;
  /** When the last check finished, in ms since the epoch. */
  at: number | null;
};

const state = createStore<UpdateCheck>({ checking: false, version: null, error: null, at: null });

export function checkForUpdate() {
  state.set({ ...state.get(), checking: true });
  call("check_update").then(
    (version) => state.set({ checking: false, version, error: null, at: Date.now() }),
    (e) => state.set({ ...state.get(), checking: false, error: errorMessage(e), at: Date.now() }),
  );
}

export const useUpdateCheck = () => useStore(state);
