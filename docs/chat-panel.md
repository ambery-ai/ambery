# Chat Panel Design

> Concept definitions are in concepts.md §3a. This document specifies invocation/dismissal, layout, and message rendering rules.

## Product Principles

Chat is the product interface for users to converse with their pet; it is not a debugging projection of Context, Queue, the streaming protocol, or the window implementation. Interaction design and acceptance should first answer what the user intends at this moment: is the user reading history, waiting for a new reply, continuing to compose input, or wanting to see the latest content right away; the underlying state is only responsible for reliably implementing that intent and must not in turn dictate the user experience.

Therefore, new messages, streaming increments, Context refreshes, window size changes, or Queue scheduling must not unilaterally change what the user is doing. Any internal system state must be translated into user-understandable feedback; the user must not be required to understand Queue, Context roles, tool calls, or a particular DOM update.

## Invocation and Dismissal

- **Invoke/Dismiss**: right-click toggle on the View (`chat:toggle`) — concepts §3a "invoked via View right-click"; when the panel is open, right-clicking again closes it. The pet stays in place (no snapping teleport; docs/view.md §Gestures and Chat invocation).
- The × in the top-right corner is the same close action (user intent to close); after ×, right-clicking again re-invokes.
- Invocation position: placed beside the pet with the fixed `sse` direction by the window placement engine `engine.place` (docs/window-positioning.md — our own positioning engine, not OS snapping).
- The panel is not a Component: it does not go through `call_component`, does not enter the Component layer, and has no direction-selection logic.

## Layout

- The panel expands attached to the pet with the fixed `sse` direction via `engine.place` (docs/window-positioning.md).
- Size 320×380. No clamping (docs/window-follow.md §Off-screen and overlap: not covering the person > fully visible; partially off-screen is accepted).

## Message Model (Projection of Context)

- Conversation history is read from Context (concepts §3a); the panel is a **view projection** of Context and holds no data of its own.
- Of the four Context roles (concepts §10b), the panel renders only the content of `user` and `assistant`; `system` (event messages) and `tool` (execution results) are runtime messages and do not pollute the conversation view (design decision).
- User input → `bridge.appendUserMessage(text)` writes into the Queue for admission → after admission it enters Context as a `user` role message → triggers one round of LLM processing (see docs/harness.md for trigger logic).
- The panel subscribes to incremental refreshes via `onContextChanged`.

## Scrolling and New Messages

Chat scrolling is determined by **user intent**, not by any single DOM update. The panel is always in one of the following states:

| State | Entry condition | On new message, streaming increment, or history refresh | Exit condition |
|---|---|---|---|
| Follow latest | First open; user sends a message; user actively scrolls back to the bottom; clicks the new-message indicator | Stays pinned to the bottom | User actively scrolls up away from the bottom |
| Reading history | User actively scrolls up away from the bottom | Keeps the current reading position, no auto-scroll; accumulates the new-message indicator | User scrolls back to the bottom or clicks the indicator |

Specific rules:

- On first open, Chat positions at the bottom of the history and enters "Follow latest".
- Sending a message indicates that the user intends to wait for subsequent content; regardless of whether the user was reading history before, after sending it unconditionally scrolls to the bottom and resumes "Follow latest".
- In "Follow latest", user messages, assistant streaming increments, assistant completion messages, and Context refreshes all stay pinned to the bottom.
- In "Reading history", no new content may steal the current viewport; the bottom of the message area shows a "↓ N new messages" indicator. One assistant reply being streamed counts as a single new message, not counted again for each increment.
- Clicking the indicator scrolls to the bottom, clears the count, and resumes "Follow latest"; manually scrolling back to the bottom has the same effect.
- A full Context re-render or a window size change must not change the user's reading intent: those following latest stay at the bottom; those reading history restore their position anchored by the first visible message before the refresh, rather than mechanically reusing the old `scrollTop`.

## Input and Send

The input area serves the user intent of "composing a message and deciding to send", not merely capturing an Enter key:

- Use an auto-growing multiline input: one line by default, growing to an explicit maximum height; beyond the maximum the input area itself scrolls and must not squeeze out the message history.
- `Enter` sends; `Shift + Enter` inserts a newline; while IME composition is not yet confirmed, `Enter` only confirms the IME candidate and must not accidentally send the message.
- The send button has exactly the same semantics as `Enter`: enabled only when non-empty content exists; blank content is not sent.
- After sending, the message immediately appears in the conversation as a user bubble; the input area clears, keeps focus, and scrolls to the bottom per the "Follow latest" rule. The user does not need to wait for the assistant to finish the current reply before continuing to type.
- Chat allows consecutive sends. While the assistant is generating, newly sent messages can still be submitted to the underlying queue; the UI must translate "replying" and "queued for processing" into user-understandable states, and must not disguise a queued message as already read by the assistant.
- When sending fails, the text the user typed must not be silently lost; the UI must clearly explain the failure and offer a path to retry or continue editing.

## Reply Indicator

- When the assistant has not yet output any body text, the latest message position shows a concise three-dot (`…`) reply indicator; it only means "responding" and does not expose internal state such as Queue, Context, tool calls, or reasoning.
- The three dots may use a lightweight animation to let the user know the interface is still active; the animation must not change layout, steal reading focus, or add extra status copy.
- The indicator disappears when the assistant starts outputting body text, completes the reply, or fails this round; process states such as "completed" are not retained in normal history.

## Message Format

```ts
interface ContextMessage {
  role: 'user' | 'assistant' | 'tool' | 'system';
  content: string;
  ts: number; // epoch ms
}
```

The assistant's `tool_calls` are not carried inside `content` (in the OpenAI model, `tool_calls` is a separate field), and the panel does not need to be aware of them.
