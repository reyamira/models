# Design System: models — Landing Page

**Project ID:** 194825300356652245

## 1. Visual Theme & Atmosphere

**Creative North Star: "The Mission Control Dashboard"**

The aesthetic is dense, utilitarian, and precise — a command center for browsing AI infrastructure. It rejects the softness of modern SaaS marketing. Instead of friendly illustrations and gradient blobs, the page presents raw data density: neon readouts on obsidian panels, CRT-style scan lines, and monospace classification labels.

The atmosphere is **cold, technical, and deliberately mechanical** — like a systems operator's terminal rendered as a website. The "sci-fi" is understated: no starfields, no hologram effects, just the language of status monitors, data grids, and terminal emulators.

**Density over elegance.** Every element earns its pixel. Decorative elements must represent data or reinforce the terminal metaphor — never exist solely for visual fill. Whitespace is intentional and asymmetric, creating rhythmic tension rather than centered comfort.

**Anti-Patterns (hard rules):**

- No gradient blobs, mesh backgrounds, or glassmorphism
- No rounded corners on structural containers — all content `border-radius: 0`. Exception: terminal chrome dots use `rounded-full` and tooltips use `rounded` as these are small decorative/utility elements, not layout containers
- No emoji — use monospace labels and terminal notation. Exception: the hero prompt uses `❯` (U+276F) as a terminal-style chevron
- No symmetrical card grids for content sections — asymmetric column splits preferred. Exception: Commands (`md:grid-cols-3`) and Footer (`md:grid-cols-4`) use equal columns where dense data grids justify it
- No generic stock illustrations — show the actual product
- No gradient-filled text — text is solid white or neon accent
- No box-shadow depth — depth is achieved through tonal layering and borders
- No decorative elements without data meaning

## 2. Color Palette & Roles

### Primary Surfaces

| Descriptive Name     | Hex                         | Functional Role                                                                                                                                                 |
| -------------------- | --------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Deep Naval Slate** | `#0f172a` (`--bg-slate`)    | Canvas background — the base "void" everything sits on. Applied to `<body>` with a subtle 40px cyan grid overlay at 5% opacity creating the graph-paper effect. |
| **Smoky Panel**      | `bg-slate-900/50`           | Container surfaces — stat cards, feature panels, command blocks. Semi-transparent to let the grid bleed through subtly.                                         |
| **Terminal Black**   | `bg-black` or `bg-black/40` | Deep-recessed containers — video panels, code blocks, terminal chrome. Creates the "sunken monitor" effect.                                                     |

### Neon Accent Triad

Three neon accents, each with a strict functional assignment. They are never interchangeable.

| Descriptive Name   | Hex                          | Functional Role                                                                                                                                                                                                                                                          |
| ------------------ | ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Electric Cyan**  | `#22d3ee` (`--neon-cyan`)    | Primary system accent — focus states, active tab indicators, data highlights, links, section headers, glow effects. The "power-on" color. Used for `.data-border` (20% opacity), `.data-header` backgrounds (10% opacity), and the background grid pattern (5% opacity). |
| **Hot Magenta**    | `#f472b6` (`--neon-magenta`) | Command/CLI accent — CLI command labels (`COMMAND_FILTER`), top-border accents on command cards, text selection highlight, the "Browse the AI ecosystem" tagline. Signals "input" and "action."                                                                          |
| **Terminal Green** | `#4ade80` (`--neon-green`)   | Install/CTA accent — install buttons, the hero install prompt (`>`), the "System Access" block, positive/go states. Signals "execute" and "ready."                                                                                                                       |

### Text Hierarchy

| Descriptive Name | Tailwind Class    | Functional Role                                                                                                                |
| ---------------- | ----------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| **Full Bright**  | `text-white`      | Primary headings, model names, emphasized data values                                                                          |
| **Cool Silver**  | `text-slate-300`  | Body descriptions, tagline text                                                                                                |
| **Muted Steel**  | `text-slate-400`  | Secondary labels, metadata, inactive states. Minimum contrast level for readable text on dark backgrounds (WCAG AA compliant). |
| Accent colors    | `text-(--neon-*)` | Functional highlights per accent role above                                                                                    |

**Contrast rule:** Never use `text-slate-500` or darker for text that conveys meaning on the `--bg-slate` background. `text-slate-400` is the floor.

### Neon Glow

Three glow utilities (`.glow-cyan`, `.glow-magenta`, `.glow-green`) apply `text-shadow: 0 0 10px` at 50% accent opacity. Used exclusively on large display numbers in stat cards — nowhere else.

## 3. Typography Rules

### Font Families

| Font                          | Character                                                 | Role                                                                                                                                                        |
| ----------------------------- | --------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Outfit** (300, 600, 900)    | Geometric, technical sans-serif with extreme weight range | Display headings (900 black), body text (300 light), section headers (600 semibold). The "voice" of the site. System sans-serif stack provides fallback.    |
| **JetBrains Mono** (400, 700) | Developer-grade monospace                                 | CLI commands, install strings, data classification labels (`Model_Density`, `COMMAND_FILTER`), terminal chrome labels, footer tech specs. The "data" voice. |

### Type Scale

| Name                | Size / Style                                                           | Usage                                                                                                  |
| ------------------- | ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| **Display**         | `clamp(60px, 15vw, 180px)` / Black / tracking-tighter / `text-balance` | Hero title ("models.") only. Fluid scaling, never fixed.                                               |
| **Section Heading** | `text-4xl` / Black / tracking-tighter / uppercase                      | Section labels ("Operations", "System Access")                                                         |
| **Feature Body**    | `text-2xl` or `text-xl` / Light / leading-tight                        | Feature descriptions in tab content and hero tagline                                                   |
| **UI Text**         | `text-sm` / `text-xs`                                                  | Navigation links, button labels, list items                                                            |
| **Data Label**      | `font-mono text-[10px] tracking-widest uppercase`                      | Terminal-style metadata labels. Used 20+ times across components. Always paired with `text-slate-400`. |
| **Code**            | `font-mono text-sm` / `font-mono text-xs`                              | CLI commands, install strings                                                                          |

### Typographic Details

- `font-variant-numeric: tabular-nums` on stat display numbers for consistent digit width
- `text-balance` on the hero heading to prevent orphaned words on reflow
- Stat numbers use the glow utility matching their accent color

## 4. Component Stylings

### Terminal Chrome

The signature "monitor frame" wrapping video panels and the hero screenshot:

- **Header bar** (`data-header`): 3 circles (`h-2 w-2 rounded-full bg-slate-700`) left-aligned + monospace uppercase label right-aligned (e.g., `MODE: TUI // TAB: MODELS`). Background `rgba(34, 211, 238, 0.1)` with bottom border at 30% opacity.
- **Container**: `data-border` class — `1px solid rgba(34, 211, 238, 0.2)`. Black background. Content below the header bar.
- All terminal chrome elements are `aria-hidden="true"`.

### Stat Cards

Four cards in a vertical stack, each using Bearnie Card (`rounded-none` override) with accent-colored borders:

- Data label in JetBrains Mono at top, large display number with accent glow at bottom-left
- Interactive graphic positioned center-right via `clamp()` responsive sizing
- Card 1 (cyan): PixiJS procedural galaxy with spiral particles, black hole sphere, perspective tilt. Hover: particle twinkle. Click: burst spin
- Card 2 (magenta): SVG scatter plot with anime.js dot pop-in. Hover: brighten. Click: re-randomize positions
- Card 3 (green): cobe WebGL globe with provider city markers. `border-l-4` green accent. Hover: speed up rotation. Click: outage flash (red→green)
- Card 4 (amber): Lottie-exported robot SVG with anime.js secondary motion (phase-offset body parts). Hover: eye blink. Click: swap through 11 bot variants
- Scroll-triggered activation via anime.js `onScroll`, staggered card fade-in + number count-up
- All WebGL/animation loops pause via IntersectionObserver when off-screen

### Feature Tabs (Bearnie Vertical Tabs)

- Tab triggers: `data-border`, left border accent (4px cyan when active, transparent when inactive), `bg-(--neon-cyan)/10` active background
- Tab content: Terminal chrome header + autoplay video with gradient overlay (`from-black/90 via-black/30 to-transparent`) + text overlaid at bottom-left
- 2px cyan progress bar at bottom of each trigger, animated via `requestAnimationFrame` synced to video playback
- Auto-cycles through tabs on video end; user click stops cycling

### Copy-to-Clipboard Buttons

Used on install cards (Install.astro) and hero command (Hero.astro). The copy script lives in Install.astro's `<script>` tag but handles ALL `[data-copy-btn]` elements on the page, including the hero button — a shared global handler.

- `<button>` with `data-copy-btn` + `data-copy-text` attributes
- Clipboard SVG icon transitions to checkmark: 200ms CSS opacity transition for the swap, 1.8s JS `setTimeout` to revert back
- Bearnie toast notification ("Copied!") on success via `window.toast?.success?.()`
- CSS-only "Click to copy" tooltip via `group-hover:opacity-100` + `group-focus-visible:opacity-100` (keyboard accessible)

### Command Cards

Three cards in a horizontal grid:

- `border-t-4` in Hot Magenta accent
- Monospace label (e.g., `COMMAND_FILTER`) in magenta
- CLI command in white monospace
- Description in data-label style below

### Navigation

- **Header**: Sticky (`sticky top-0`), backdrop-blur. Cyan square pulse indicator (`h-3 w-3`, not rounded — a deliberate square) + "System: Models_OS" label left-aligned. Three bracketed nav links right-aligned in monospace uppercase: `[ documentation ]` (cyan), `[ source_code ]` (slate-400), `[ license ]` (slate-400). All links use centralized URLs from `src/data/site.ts`.
- **Footer**: Four-column grid. Column 1: "MODELS" brand + tech stack specs in monospace. Column 2: Navigation links (repo, docs, crates.io) + version. Column 3: Environment info (OS, license). Column 4: Dynamic copyright year.

### Hero Screenshot

Separate from the feature tab videos. Uses Astro `<Image>` component with optimized WebP output (`src/assets/` ESM import, `loading="eager"`). Gradient overlay fades to `var(--bg-slate)` (matching the page background), unlike feature tab overlays which fade to black. Terminal chrome + "LIVE_SYNC" badge + version string overlay.

### Install Section

Five copy-to-clipboard buttons in a responsive grid (Homebrew, Cargo, Nix, Scoop, AUR) + a "GitHub_Releases" link with arrow below. The "Installation Commands" heading uses `italic` — the only italic text on the site, for visual differentiation of the CTA block.

## 5. Layout Principles

### Asymmetric Grid Philosophy

Layouts use intentionally unequal column splits to create visual tension:

| Section      | Grid              | Split Ratio                        |
| ------------ | ----------------- | ---------------------------------- |
| Hero + Stats | `lg:grid-cols-12` | 8:4 (hero dominates)               |
| Feature Tabs | `md:grid-cols-4`  | 1:3 (sidebar:content)              |
| Install Grid | `lg:grid-cols-4`  | 1:3 (label:cards)                  |
| Commands     | `md:grid-cols-3`  | Equal (exception: dense data grid) |
| Footer       | `md:grid-cols-4`  | Equal (exception: dense data)      |

### Spacing Rhythm

- Between sections: `space-y-8` (32px)
- Between cards within a section: `gap-4` (16px)
- Card internal padding: `p-6` (small cards) or `p-8` (large panels)

### Depth Without Shadows

Depth is achieved exclusively through tonal layering:

1. Canvas (`--bg-slate`) — the deepest layer
2. Panels (`bg-slate-900/50`) — mid-layer containers
3. Recessed (`bg-black`) — terminal screens, video panels
4. Accent highlights — `data-header` background at 10% cyan

No `box-shadow` in authored code. The 1px `data-border` is the only boundary mechanism. Note: the base Bearnie `TabsTrigger.astro` component carries a latent `data-[state=active]:shadow-sm` class from its defaults, but this is suppressed by the override classes in `Features.astro`'s `tabTriggerClass`.

## 6. Motion & Animation

### Reduced-Motion Gated

These animations respect `prefers-reduced-motion`:

- **Scanline overlay** (`.active-scanline::after`): Wrapped in `@media (prefers-reduced-motion: no-preference)` in global.css. Repeating 4px horizontal gradient creating CRT scan line effect. Purely cosmetic, `pointer-events: none`.
- **Pulse dot** (`motion-safe:animate-pulse`): Header status indicator. Uses Tailwind `motion-safe:` prefix. Signals "system active."

### Not Yet Reduced-Motion Gated

These animations run unconditionally (known gap):

- **Tab auto-cycle**: Videos play and cycle through tabs regardless of motion preference. The `requestAnimationFrame` progress bar animation also runs unconditionally. Should be gated in a future pass.
- **Icon transitions**: Clipboard-to-checkmark swap uses 200ms CSS opacity transition. Subtle enough to be acceptable, but could be made instant under reduced-motion.

### Stat Card Animations

Stat cards use `animejs` v4 for scroll-triggered entry, number counters, hover effects, and click easter eggs. Each card has a unique visualization:

- **Model_Density**: galaxy SVG (recolored from LottieFiles)
- **Bench_Validation**: programmatic scatter plot (SVG dots animated by anime.js)
- **Provider_Nodes**: cobe WebGL globe with real provider city markers
- **Agent_Tracker**: Lottie bot exported as static SVG with per-part secondary motion

**Secondary motion pattern**: give each animated body part a slightly different duration (e.g., body=2500ms, head=2800ms, arms=2900-3100ms) so parts phase in and out of sync naturally. All use `inOutSine` easing for floating feel.

**Robot easing rule**: no elastic/springy easings for mechanical characters. Use `inOutSine` for floating, `inQuad`/`outQuad` for deliberate movements. Linear feels too rigid, elastic feels too organic.

### Motion Philosophy

No spring physics, no parallax. Scroll-triggered activation is used for stat cards (staggered reveal + count-up). Motion is functional (indicating state, revealing content on scroll) or atmospheric (globe rotation, galaxy drift), not gratuitous.
