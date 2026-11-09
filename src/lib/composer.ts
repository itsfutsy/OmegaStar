// The conversation's menu asks its composer for things it cannot reach: words to start from ("Teach a new task") and
// the file picker. Each composer listens for its own conversation's draft key.
import { useEffect, useRef } from "react";

type ComposerAsk = { type: "prefill"; text: string } | { type: "attach" };

const EVENT = "omega:composer";

/** Asks the composer of the conversation whose draft key is `key`. */
export function askComposer(key: string, ask: ComposerAsk) {
  window.dispatchEvent(new CustomEvent(EVENT, { detail: { key, ask } }));
}

/** Answers what is asked of the composer whose draft key is `key`. */
export function useComposerAsks(key: string, on: (ask: ComposerAsk) => void) {
  const handler = useRef(on);
  handler.current = on;
  useEffect(() => {
    const listen = (ev: Event) => {
      const { detail } = ev as CustomEvent<{ key: string; ask: ComposerAsk }>;
      if (detail.key === key) handler.current(detail.ask);
    };
    window.addEventListener(EVENT, listen);
    return () => window.removeEventListener(EVENT, listen);
  }, [key]);
}
