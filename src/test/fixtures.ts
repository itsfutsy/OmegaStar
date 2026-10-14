// Small builders for the pure-module tests.
import type { Run, Thread } from "../api/types";

export function run(over: Partial<Run> = {}): Run {
  return {
    id: "r1", employeeId: "e1", kind: { type: "manual" }, prompt: "", state: "succeeded",
    createdAt: "2026-10-01T09:00:00Z", startedAt: "2026-10-01T09:00:01Z", endedAt: "2026-10-01T09:05:00Z",
    turns: 1, durationMs: 299_000, summary: null, error: null, denials: [], skillChanges: [], needsYou: [],
    archivedAt: null, repliedAt: null, deliveredAt: null, allowOnce: [], proposed: null, attachments: [], build: null,
    ...over,
  };
}

export const thread = (root: Run, replies: Run[] = [], related: Thread[] = []): Thread => ({ root, replies, related });
