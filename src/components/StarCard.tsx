import { useState } from "react";
import { call, errorMessage } from "../api/client";
import { useOrg } from "../lib/hooks";
import { toast } from "../lib/toasts";
import { useUpdateCheck } from "../lib/update";
import { openWeb } from "./conversation/CardLink";
import { Icon } from "./Icon";
import "./StarCard.css";

/** The card's words and the repository it opens, kept here so a change lands in one place. */
const TITLE = "Hey, I'm Futsy";
const LINE = "I make OmegaStar. It's free and open source so a star on GitHub would be awesome.";
const BUTTON = "Star on GitHub";
const DISMISS = "No thanks";
const RAIL = "Star OmegaStar on GitHub";
const REPO = new URL("https://github.com/itsfutsy/OmegaStar");

/** The sidebar's one-time ask to star OmegaStar on GitHub, once you have sent enough messages of your own (`starAsk`).
 *  Starring opens the repository in your browser; either button answers it, and it never comes back. It waits until
 *  the update check has answered and while an update is offered, so the sidebar holds one card at a time. On the rail
 *  it is a star button. */
export function StarCard() {
  const { snap } = useOrg();
  const { version, at } = useUpdateCheck();
  const [answered, setAnswered] = useState(false);

  if (!snap?.starAsk || answered || at === null || version) return null;
  const answer = () => {
    setAnswered(true);
    call("answer_star_ask").catch((e) => toast(errorMessage(e), "err"));
  };
  const star = () => {
    openWeb(REPO);
    answer();
  };

  return (
    <>
      <div className="update star-card" aria-label={RAIL}>
        <div className="update-title">{TITLE}</div>
        <div className="update-sub">{LINE}</div>
        <button className="btn btn-sm update-btn star-btn" onClick={star}><Icon name="star" size={13} />{BUTTON}</button>
        <button className="star-x" onClick={answer} aria-label={DISMISS} data-tip={DISMISS}><Icon name="close" size={13} /></button>
      </div>
      <button className="update-mini star-mini" onClick={star} aria-label={RAIL} data-tip={RAIL}>
        <Icon name="star" size={16} />
      </button>
    </>
  );
}
