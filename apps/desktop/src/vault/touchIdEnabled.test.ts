import { TOUCH_ID_ENABLED } from "./touchIdEnabled";

// v0.4.2 turns the Touch ID controls back on (personal-cfo-xuu): v0.4.1 hid them while turning Touch ID on failed on
// every Mac with a sensor. This is the ONE constant that release flips. This test runs with the real module, never a
// mock, so it pins what a build actually contains; changing the constant means changing this test in the same commit.
describe("the shipped Touch ID switch", () => {
  it("is on in this release", () => {
    expect(TOUCH_ID_ENABLED).toBe(true);
  });
});
