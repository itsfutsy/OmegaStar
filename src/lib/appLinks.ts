// `omegastar://` links: a link opened outside the app ("Chat with Hugo about this" in a pull request) brings the
// window forward, and this opens the conversation it names. A link only opens a conversation and may write a first
// line for you to send or change; it never sends anything.
import { useEffect } from "react";
import { call, errorMessage, subscribe } from "../api/client";
import type { LinkTarget } from "../api/types";
import { newConversationDraft } from "../components/conversation/NewConversation";
import { getDraft, setDraft } from "./drafts";
import { navigate } from "./router";
import { threadFocus } from "./threads";
import { toast } from "./toasts";

/** Opens where a link points. A first line never replaces one you have already started writing. */
export function openLink(target: LinkTarget) {
  if (target.type === "thread") {
    navigate({ page: "chat", employeeId: target.employeeId, focus: threadFocus(target.rootId) });
    return;
  }
  const key = newConversationDraft(target.employeeId);
  if (target.draft && !getDraft(key).trim()) setDraft(key, target.draft);
  navigate({ page: "chat", employeeId: target.employeeId, focus: null });
}

/** Reads the waiting link when the app opens (a link that started it) and each time one is opened while it runs. */
export function useAppLinks() {
  useEffect(() => {
    const take = () =>
      call("take_app_link")
        .then((t) => t && openLink(t))
        .catch((e) => toast(errorMessage(e), "err"));
    void take();
    return subscribe("link:opened", () => void take());
  }, []);
}
