import { MAX_NOTICES, type ActiveNotice } from "@/components/ui/notice-context";
import { addNotice, EMPTY_STACK, removeNotice, type NoticeStack } from "@/components/ui/notice-stack";

// The rule for a full stack (docs/agent/FRONTEND.md 6.2), without a DOM.
const undo = async () => true;
const undoNotice = (key: number): ActiveNotice => ({ key, message: `undo ${key}`, undo });
const errorNotice = (key: number): ActiveNotice => ({ key, message: `error ${key}` });
const keys = (stack: NoticeStack) => stack.shown.map((n) => n.key);
const build = (...notices: ActiveNotice[]) => notices.reduce(addNotice, EMPTY_STACK);

test("the limit is three", () => {
  expect(MAX_NOTICES).toBe(3);
});

test("with room, notices show newest last", () => {
  expect(keys(build(undoNotice(1), errorNotice(2), undoNotice(3)))).toEqual([1, 2, 3]);
});

test("full: the oldest UNDO notice is evicted, wherever it sits", () => {
  expect(keys(build(errorNotice(1), undoNotice(2), undoNotice(3), undoNotice(4)))).toEqual([1, 3, 4]);
  expect(keys(build(undoNotice(1), errorNotice(2), errorNotice(3), errorNotice(4)))).toEqual([2, 3, 4]);
});

test("full: an error is never evicted, even when it is the oldest", () => {
  const stack = build(errorNotice(1), undoNotice(2), undoNotice(3), undoNotice(4), undoNotice(5));
  expect(keys(stack)).toEqual([1, 4, 5]);
  expect(stack.waiting).toEqual([]);
});

test("full of errors: the new notice waits (nothing is lost)", () => {
  const stack = build(errorNotice(1), errorNotice(2), errorNotice(3), undoNotice(4), errorNotice(5));
  expect(keys(stack)).toEqual([1, 2, 3]);
  expect(stack.waiting.map((n) => n.key)).toEqual([4, 5]);
});

test("closing one lets the waiting ones in, oldest first, while there is room", () => {
  const full = build(errorNotice(1), errorNotice(2), errorNotice(3), undoNotice(4), undoNotice(5));
  const afterOne = removeNotice(full, 2);
  expect(keys(afterOne)).toEqual([1, 3, 4]);
  expect(afterOne.waiting.map((n) => n.key)).toEqual([5]);
  const afterTwo = removeNotice(afterOne, 1);
  expect(keys(afterTwo)).toEqual([3, 4, 5]);
  expect(afterTwo.waiting).toEqual([]);
});

test("closing a waiting notice that was never shown removes it", () => {
  const full = build(errorNotice(1), errorNotice(2), errorNotice(3), undoNotice(4));
  const stack = removeNotice(full, 4);
  expect(stack.waiting).toEqual([]);
  expect(keys(stack)).toEqual([1, 2, 3]);
});

test("closing an unknown key changes nothing", () => {
  const stack = build(undoNotice(1));
  expect(removeNotice(stack, 99)).toEqual(stack);
});
