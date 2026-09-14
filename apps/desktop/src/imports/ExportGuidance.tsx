// The per-institution export walkthrough (personal-cfo-rfsc / lu4tm): pick a
// bank, read the landmarks, download, import. Pure presentation over the
// static registry; hosts decide where it sits (onboarding, the import dialog).

import { useState } from "react";

import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";

import { EXPORT_GUIDES, FORMAT_LABEL, guideFor } from "./exportGuides";

export function ExportGuidance({
  id = "export-guide",
  defaultId = "other",
}: {
  id?: string;
  /// Preselect an institution (the import-freshness item can name one, o7w0).
  defaultId?: string;
}) {
  const [selected, setSelected] = useState(defaultId);
  const guide = guideFor(selected);
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={id} className="text-sm">
          Where is the money?
        </Label>
        <NativeSelect
          id={id}
          size="sm"
          value={selected}
          onChange={(event) => setSelected(event.target.value)}
        >
          {EXPORT_GUIDES.map((entry) => (
            <option key={entry.id} value={entry.id}>
              {entry.name}
            </option>
          ))}
        </NativeSelect>
      </div>
      <ol className="flex list-decimal flex-col gap-1 pl-5 text-sm">
        {guide.steps.map((step, index) => (
          <li key={index}>{step}</li>
        ))}
      </ol>
      <p className="text-xs text-muted-foreground">
        Formats {guide.id === "other" ? "to look for" : "usually offered"}:{" "}
        {guide.formats.map((format) => FORMAT_LABEL[format]).join(", ")}.
        {guide.note ? ` ${guide.note}` : ""} Menus move; the landmarks above are
        where the export usually lives.
      </p>
    </div>
  );
}
