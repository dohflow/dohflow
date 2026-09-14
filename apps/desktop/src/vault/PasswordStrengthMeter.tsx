import { cn } from "@/lib/utils";

/// 0–4 strength bucket from a local heuristic — length plus character variety.
/// Intentionally simple and dependency-free (a zxcvbn-style estimator is deferred,
/// personal-cfo-00xl). The candidate password never leaves the client.
function passwordStrength(password: string): number {
  if (password.length === 0) return 0;
  let score = 0;
  if (password.length >= 8) score += 1;
  if (password.length >= 12) score += 1;
  if (/[0-9]/.test(password) && /[a-zA-Z]/.test(password)) score += 1;
  if (/[^a-zA-Z0-9]/.test(password)) score += 1;
  return score;
}

const LABELS = ["", "Weak", "Fair", "Good", "Strong"] as const;

/// The label + the *literal* Tailwind classes for a score (1 weak, 2 fair, 3–4
/// good). Literal strings so Tailwind's scanner keeps them — never build class
/// names by interpolation.
function band(score: number): { bar: string; text: string } {
  if (score >= 3) return { bar: "bg-gain", text: "text-gain" };
  if (score === 2) return { bar: "bg-warning", text: "text-warning" };
  return { bar: "bg-loss", text: "text-loss" };
}

/// Visual + textual password-strength feedback (personal-cfo-00xl): four bars plus
/// a word (Weak / Fair / Good / Strong). Reusable across vault create and a future
/// change-password flow (blocked on key rotation, personal-cfo-2y8). Hidden while
/// the field is empty.
export function PasswordStrengthMeter({ password }: { password: string }) {
  const score = passwordStrength(password);
  const { bar, text } = band(score);

  return (
    <div className="flex flex-col gap-1">
      <div className="flex gap-1.5" aria-hidden>
        {[0, 1, 2, 3].map((i) => (
          <div
            key={i}
            className={cn("h-1 flex-1 rounded-full", i < score ? bar : "bg-muted")}
          />
        ))}
      </div>
      {score > 0 && (
        <p className="text-xs text-muted-foreground">
          Password strength:{" "}
          <span className={cn("font-medium", text)}>{LABELS[score]}</span>
        </p>
      )}
    </div>
  );
}
