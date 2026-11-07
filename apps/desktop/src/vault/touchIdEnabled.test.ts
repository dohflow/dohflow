import { TOUCH_ID_ENABLED } from "./touchIdEnabled";

// v0.4.1 ships with every Touch ID control hidden (personal-cfo-p3f7r): turning Touch ID on failed on every Mac with a sensor in
// the 0.4.0 build (personal-cfo-xuu). This is the ONE constant that v0.4.2 flips. This test runs with the real module, never a
// mock, so it pins what a build actually contains; flipping the constant means changing this test in the same commit.
describe("the shipped Touch ID switch", () => {
  it("is off in this release", () => {
    expect(TOUCH_ID_ENABLED).toBe(false);
  });
});
