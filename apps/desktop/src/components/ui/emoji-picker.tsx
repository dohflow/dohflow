import * as React from "react";
import { ChevronDown, X } from "lucide-react";

import { cn } from "@/lib/utils";

/// Curated, dep-free emoji set (personal-cfo-4d8.25.20): grouped for scanning,
/// keyword-tagged for filtering. Typing filters the grid but can never be
/// committed as a value — selection is click (or Enter on a focused cell) only.
const EMOJI_GROUPS: Array<{ name: string; emoji: Array<[string, string]> }> = [
  {
    name: "Money",
    emoji: [
      ["💰", "money bag savings"],
      ["💵", "cash dollar bill"],
      ["💳", "credit card payment"],
      ["🏦", "bank account"],
      ["🪙", "coin change"],
      ["📈", "chart growth invest stocks"],
      ["📉", "chart loss decline"],
      ["🧾", "receipt bill tax"],
      ["💸", "money wings spend fees"],
      ["🤑", "rich money face"],
    ],
  },
  {
    name: "Home",
    emoji: [
      ["🏠", "house home rent mortgage"],
      ["🏢", "building apartment office"],
      ["🛋️", "couch furniture living"],
      ["🛏️", "bed bedroom furniture"],
      ["🧹", "broom cleaning chores"],
      ["🧺", "laundry basket"],
      ["🔧", "wrench repair maintenance"],
      ["🔨", "hammer tools renovation"],
      ["💡", "light bulb electricity utilities"],
      ["🔥", "fire gas heating"],
      ["💧", "water drop utilities"],
      ["🌡️", "thermometer heating cooling hvac"],
      ["📶", "wifi internet signal"],
      ["📱", "phone mobile cell"],
      ["🖥️", "computer desktop electronics"],
      ["🧴", "lotion toiletries supplies"],
      ["🪴", "plant garden"],
      ["🐶", "dog pet"],
      ["🐱", "cat pet"],
    ],
  },
  {
    name: "Food",
    emoji: [
      ["🛒", "groceries shopping cart supermarket"],
      ["🍎", "apple fruit produce"],
      ["🥦", "broccoli vegetables produce"],
      ["🍞", "bread bakery"],
      ["🥛", "milk dairy"],
      ["🍗", "chicken meat"],
      ["🍕", "pizza takeout"],
      ["🍔", "burger fast food"],
      ["🌮", "taco mexican"],
      ["🍣", "sushi japanese"],
      ["🍜", "noodles ramen"],
      ["🥗", "salad healthy"],
      ["☕", "coffee cafe espresso"],
      ["🍺", "beer bar drinks"],
      ["🍷", "wine drinks"],
      ["🧋", "boba bubble tea"],
      ["🍩", "donut dessert sweets"],
      ["🎂", "cake birthday dessert"],
      ["🍽️", "dining restaurant eating out"],
    ],
  },
  {
    name: "Transport",
    emoji: [
      ["🚗", "car auto vehicle"],
      ["⛽", "gas fuel petrol"],
      ["🔌", "charging electric ev"],
      ["🅿️", "parking"],
      ["🚌", "bus transit"],
      ["🚇", "metro subway train transit"],
      ["🚆", "train rail"],
      ["🚕", "taxi rideshare uber lyft"],
      ["🚲", "bike bicycle cycling"],
      ["🛴", "scooter"],
      ["🏍️", "motorcycle"],
      ["🛞", "tire wheel maintenance"],
      ["🧰", "toolbox repair service"],
      ["🛣️", "highway tolls road"],
    ],
  },
  {
    name: "Health",
    emoji: [
      ["🏥", "hospital medical"],
      ["💊", "pills medicine pharmacy prescription"],
      ["🩺", "stethoscope doctor checkup"],
      ["🦷", "tooth dentist dental"],
      ["👓", "glasses vision optometrist"],
      ["🧠", "brain therapy mental health"],
      ["🏋️", "weights gym fitness workout"],
      ["🧘", "yoga meditation wellness"],
      ["🏃", "running exercise"],
      ["🩹", "bandage first aid"],
      ["🧬", "dna lab tests"],
      ["🤱", "baby childcare"],
    ],
  },
  {
    name: "Fun",
    emoji: [
      ["🎬", "movie cinema film streaming"],
      ["🎮", "gaming video games"],
      ["🎵", "music spotify streaming"],
      ["📺", "tv television streaming subscription"],
      ["📚", "books reading"],
      ["🎨", "art hobby craft"],
      ["🎸", "guitar music instrument"],
      ["⚽", "soccer sports"],
      ["🏈", "football sports"],
      ["⛳", "golf sports"],
      ["🎿", "ski snow sports"],
      ["🏕️", "camping outdoors"],
      ["🎣", "fishing outdoors"],
      ["🎟️", "tickets events concert show"],
      ["🎉", "party celebration"],
      ["🎁", "gift present"],
    ],
  },
  {
    name: "Life",
    emoji: [
      ["👕", "clothes shirt clothing apparel"],
      ["👟", "shoes sneakers"],
      ["💄", "makeup beauty cosmetics"],
      ["💇", "haircut salon barber"],
      ["🧼", "soap personal care hygiene"],
      ["📦", "package amazon delivery shopping"],
      ["🛍️", "shopping bags retail"],
      ["💍", "ring jewelry"],
      ["👶", "baby kids children"],
      ["🎓", "graduation education tuition school"],
      ["✏️", "pencil school supplies education"],
      ["🧸", "toys kids"],
      ["⛪", "church donation tithe"],
      ["❤️", "heart charity giving donation"],
    ],
  },
  {
    name: "Work & travel",
    emoji: [
      ["💼", "briefcase work business"],
      ["🖊️", "pen office supplies"],
      ["📊", "presentation business reports"],
      ["✈️", "plane flight travel"],
      ["🏨", "hotel lodging travel"],
      ["🧳", "luggage suitcase travel"],
      ["🗺️", "map travel vacation"],
      ["🏖️", "beach vacation"],
      ["🚢", "cruise ship travel"],
      ["🛂", "passport visa travel"],
      ["💱", "currency exchange fx"],
      ["🛡️", "shield insurance protection"],
      ["⚖️", "scales legal lawyer"],
      ["🏛️", "government taxes irs"],
      ["📮", "mail postage shipping"],
      ["🔒", "lock security subscription"],
    ],
  },
];

interface EmojiPickerProps {
  value: string;
  onSelect: (emoji: string) => void;
  onClear?: () => void;
  "aria-label": string;
  id?: string;
}

/// Click-only emoji picker (personal-cfo-4d8.25.20): the trigger shows the current
/// emoji; the panel offers a keyword filter + the curated grid. Free text can never
/// become the value — the owner's complaint was exactly that the old text input
/// accepted anything (and most people can't type emoji on a desktop keyboard).
export function EmojiPicker({ value, onSelect, onClear, id, "aria-label": ariaLabel }: EmojiPickerProps) {
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const rootRef = React.useRef<HTMLDivElement>(null);
  const inputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    if (!open) return;
    function onPointerDown(event: PointerEvent) {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setOpen(false);
        setQuery("");
      }
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.stopPropagation();
        setOpen(false);
        setQuery("");
      }
    }
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [open]);

  React.useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  const q = query.trim().toLowerCase();
  const groups = EMOJI_GROUPS.map((group) => ({
    name: group.name,
    emoji: group.emoji.filter(
      ([glyph, keywords]) => q === "" || glyph === query.trim() || keywords.includes(q),
    ),
  })).filter((group) => group.emoji.length > 0);

  function pick(glyph: string) {
    onSelect(glyph);
    setOpen(false);
    setQuery("");
  }

  return (
    <div ref={rootRef} className="relative">
      <div className="flex items-center gap-1">
        <button
          type="button"
          id={id}
          aria-label={ariaLabel}
          aria-haspopup="dialog"
          aria-expanded={open}
          onClick={() => setOpen((o) => !o)}
          className={cn(
            "flex h-9 w-14 items-center justify-center gap-1 rounded-md border border-input bg-background text-base",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
          )}
        >
          {value === "" ? (
            <span className="text-sm text-muted-foreground">🙂?</span>
          ) : (
            <span aria-hidden>{value}</span>
          )}
          <ChevronDown className="size-3 text-muted-foreground" aria-hidden />
        </button>
        {value !== "" && onClear !== undefined && (
          <button
            type="button"
            aria-label="Clear icon"
            onClick={onClear}
            className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <X className="size-3.5" aria-hidden />
          </button>
        )}
      </div>
      {open && (
        <div
          role="dialog"
          aria-label="Pick an emoji"
          // Lets an ancestor dialog's Escape handler yield to this open popover
          // instead of closing itself (adversarial review of 4d8.25.19/.20).
          data-escape-layer="popover"
          className="absolute left-0 top-full z-50 mt-1 w-72 rounded-md border bg-background shadow-lg"
        >
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            // The filter is search-only by design: Enter never commits text.
            onKeyDown={(event) => {
              if (event.key === "Enter") event.preventDefault();
            }}
            placeholder="Filter (e.g. coffee)…"
            aria-label="Filter emoji"
            className="w-full border-b bg-transparent px-3 py-2 text-sm focus:outline-none"
          />
          <div className="max-h-64 overflow-y-auto p-2">
            {groups.length === 0 && (
              <p className="px-1 py-2 text-sm text-muted-foreground">No matches</p>
            )}
            {groups.map((group) => (
              <div key={group.name}>
                <p className="px-1 pt-1 text-xs font-medium text-muted-foreground">
                  {group.name}
                </p>
                <div className="grid grid-cols-8">
                  {group.emoji.map(([glyph, keywords]) => (
                    <button
                      key={glyph}
                      type="button"
                      title={keywords}
                      aria-label={`Emoji ${glyph} ${keywords.split(" ")[0]}`}
                      onClick={() => pick(glyph)}
                      className="rounded p-1 text-lg hover:bg-accent focus-visible:bg-accent focus-visible:outline-none"
                    >
                      {glyph}
                    </button>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
