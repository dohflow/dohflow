/// Dev-only: shows another dev page inside an iframe of a chosen size, so a narrow or short window
/// can be captured from a larger browser window (a headless window has a minimum size).
/// `?src=/scenarios-revert.html%3Fstate%3Dlines&w=360&h=500`. Fake data only; not in the app bundle.
const params = new URLSearchParams(window.location.search);
const frame = document.createElement("iframe");
frame.src = params.get("src") ?? "/";
frame.width = params.get("w") ?? "360";
frame.height = params.get("h") ?? "700";
document.body.append(frame);
