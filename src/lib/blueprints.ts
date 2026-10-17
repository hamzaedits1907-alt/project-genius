// Blueprints: curated starting points for the Home gallery. Each one carries a detailed,
// buildable brief (pages, features, data, visual direction) so the first build is far better
// than what a one-line idea produces. Pure data so it can be tested and reused.

export type BlueprintCategory = 'website' | 'app' | 'game' | 'tool';

export interface Blueprint {
  id: string;
  title: string;
  /** One line shown on the card. */
  description: string;
  category: BlueprintCategory;
  tags: string[];
  /** Suggested project name when building straight from the card. */
  projectName: string;
  /** Two accent colours for the card illustration (hex). */
  colors: [string, string];
  /** The full build brief placed in the idea box. */
  prompt: string;
}

export const BLUEPRINT_FILTERS: { id: 'all' | BlueprintCategory; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'website', label: 'Websites' },
  { id: 'app', label: 'Apps' },
  { id: 'game', label: 'Games' },
  { id: 'tool', label: 'Tools' },
];

export type BlueprintFilter = (typeof BLUEPRINT_FILTERS)[number]['id'];

const MUST_HAVES =
  'fully responsive from phone to wide desktop; accessible (semantic HTML, keyboard navigation, visible focus states, good contrast, alt text); polished empty, loading and error states; subtle, purposeful motion that respects reduced-motion; no external backend, sign-in or API keys.';

/** Assembles a brief from its parts so every blueprint follows the same structure. */
function brief(b: { goal: string; pages: string; features: string[]; data: string; style: string; mustHaves?: string }): string {
  return [
    b.goal,
    `Pages & sections: ${b.pages}`,
    `Key features:\n${b.features.map((f) => `- ${f}`).join('\n')}`,
    `Data: ${b.data}`,
    `Visual style: ${b.style}`,
    `Must-haves: ${b.mustHaves ? `${b.mustHaves}; ` : ''}${MUST_HAVES}`,
  ].join('\n\n');
}

export const BLUEPRINTS: Blueprint[] = [
  // ------------------------------------------------------------------ websites
  {
    id: 'saas-landing',
    title: 'SaaS landing page',
    description: 'A conversion-focused launch page with pricing, FAQ and a waitlist.',
    category: 'website',
    tags: ['Marketing', 'Pricing', 'Waitlist'],
    projectName: 'Launch Page',
    colors: ['#7c5cff', '#ff7ab8'],
    prompt: brief({
      goal: 'A high-converting landing page for "Flowly", a fictional AI meeting-notes SaaS for remote teams. It should feel like a top-tier startup launch: clear value proposition, social proof and an obvious next step.',
      pages: 'sticky header with logo, nav links and a "Join the waitlist" button; hero with headline, sub-headline, primary + secondary CTA and an app screenshot mock built in HTML/CSS; logo strip of sample customers; three feature blocks with icons; "How it works" in three steps; testimonials carousel; pricing with a monthly/yearly toggle (Free, Pro, Team; yearly shows a discount badge); FAQ accordion; final CTA band; footer with link columns.',
      features: [
        'Waitlist form with inline email validation, success state and duplicate detection',
        'Pricing toggle animates prices and highlights the recommended plan',
        'Smooth-scrolling nav with the active section highlighted while scrolling',
        'Dark/light theme toggle that remembers the choice',
      ],
      data: 'waitlist emails and the theme preference are saved in localStorage; a small "Waitlist (sample admin view)" link shows the saved entries with an export-as-CSV button. Customer logos, testimonials and numbers are realistic sample content, visibly labelled as sample.',
      style: 'modern and premium: deep navy background, violet-to-pink gradient accents, generous whitespace, large confident typography (Inter or similar), soft glows and glassy cards.',
    }),
  },
  {
    id: 'portfolio',
    title: 'Personal portfolio',
    description: 'A bold creative portfolio with filterable projects and case studies.',
    category: 'website',
    tags: ['Personal', 'Gallery', 'Contact'],
    projectName: 'Portfolio',
    colors: ['#f2b45a', '#ff7a59'],
    prompt: brief({
      goal: 'A personal portfolio for "Alex Rivera", a fictional product designer and front-end developer, that makes a memorable first impression on recruiters and clients.',
      pages: 'home with an animated hero (name, role, short pitch, "View work" and "Contact" buttons); selected work grid; project case-study pages (problem, process, outcome, image gallery, next/previous project); about page with photo placeholder, bio, skills and an experience timeline; contact page.',
      features: [
        'Project grid filterable by category (Product, Web, Branding) with animated transitions',
        'Case studies open as routes with shareable URLs (hash routing)',
        'Contact form with validation, a friendly success message and a "copy email" button',
        'Downloadable résumé button (links to a sample PDF placeholder)',
      ],
      data: 'six sample projects and the experience timeline live in a typed data file, clearly marked as sample content; contact form messages are stored in localStorage so the flow can be demoed.',
      style: 'editorial and bold: off-white background with near-black text, one warm accent (amber/coral), oversized headings, asymmetric grid, smooth hover reveals on project cards, optional dark mode.',
    }),
  },
  {
    id: 'online-store',
    title: 'Online store',
    description: 'A product catalog with filters, cart drawer and a checkout flow.',
    category: 'website',
    tags: ['E-commerce', 'Cart', 'Checkout'],
    projectName: 'Online Store',
    colors: ['#3ecf8e', '#5aa9ff'],
    prompt: brief({
      goal: 'An online store front-end for "Terra Goods", a fictional shop selling sustainable home products, with a complete browse → cart → checkout experience (no real payments).',
      pages: 'home with hero banner, featured categories and best sellers; catalog with filters; product detail page (gallery, variants, quantity, reviews summary, related products); cart drawer; three-step checkout (shipping, delivery method, review) and an order confirmation page; order history page.',
      features: [
        'Catalog search, category and price filters, sort (price, rating, newest) and pagination',
        'Cart with quantity controls, subtotal, shipping estimate and a promo code (try "SAMPLE10")',
        'Checkout form with validation; payment is a clearly marked demo step that never asks for real card details',
        'Wishlist hearts and a "recently viewed" row',
      ],
      data: 'about 24 sample products (name, price, images via colourful placeholder gradients, rating, stock) in a typed data file labelled as sample; cart, wishlist and placed demo orders persist in localStorage.',
      style: 'clean and trustworthy: light background, earthy green accent, rounded product cards with soft shadows, crisp price typography, toast confirmations when adding to cart.',
    }),
  },
  {
    id: 'blog-magazine',
    title: 'Blog / magazine',
    description: 'An editorial magazine with categories, reading view and bookmarks.',
    category: 'website',
    tags: ['Content', 'Reading', 'Search'],
    projectName: 'Magazine',
    colors: ['#ff7ab8', '#a48bff'],
    prompt: brief({
      goal: 'A modern online magazine called "The Long Read" (fictional) for curious readers who enjoy thoughtful articles on technology, design and culture.',
      pages: 'home with a featured story, a "latest" list and category rows; category pages; article page with hero image, author byline, reading time, table of contents, pull quotes and related articles; author page; bookmarks page; search results.',
      features: [
        'Full-text search across titles, tags and article bodies with highlighted matches',
        'Reading progress bar, adjustable text size and a distraction-free reading mode',
        'Bookmark articles and see them on a dedicated page',
        'Newsletter signup box with validation',
      ],
      data: 'twelve sample articles with realistic, varied body text (written as sample content and labelled as such), authors and categories in a typed data file; bookmarks, reading preferences and newsletter signups are stored in localStorage.',
      style: 'elegant editorial: serif headlines with a clean sans-serif body, comfortable 68-character line length, generous margins, tasteful pink/violet accents, light and dark themes.',
    }),
  },
  {
    id: 'restaurant',
    title: 'Restaurant & menu',
    description: 'A warm restaurant site with an interactive menu and table reservations.',
    category: 'website',
    tags: ['Local business', 'Menu', 'Reservations'],
    projectName: 'Restaurant',
    colors: ['#f2b45a', '#d23a52'],
    prompt: brief({
      goal: 'A website for "Osteria Lume", a fictional neighbourhood Italian restaurant, that makes visitors hungry and turns them into reservations.',
      pages: 'home with a full-bleed hero, a short story section, signature dishes and testimonials; menu page with tabs (Starters, Pasta, Mains, Desserts, Drinks); reservations page; gallery; location and opening hours with a map placeholder; footer with contact details.',
      features: [
        'Menu filters for vegetarian, vegan and gluten-free with clear dietary icons and prices',
        'Reservation form with date, time slots, party size and special requests; shows a confirmation code',
        'Live "Open now / Closed" badge based on the opening hours',
        'My reservations page to view or cancel upcoming bookings',
      ],
      data: 'the menu (around 25 sample dishes with descriptions and prices), hours and testimonials come from a typed data file labelled as sample; reservations are saved in localStorage.',
      style: 'warm and inviting: cream background, deep terracotta and olive accents, elegant serif headings, food imagery represented by rich gradient placeholders, gentle fade-in on scroll.',
    }),
  },
  {
    id: 'event-conference',
    title: 'Event / conference site',
    description: 'A conference site with schedule, speakers and a personal agenda.',
    category: 'website',
    tags: ['Events', 'Schedule', 'Tickets'],
    projectName: 'Conference',
    colors: ['#5aa9ff', '#7c5cff'],
    prompt: brief({
      goal: 'A website for "FutureStack 2026", a fictional two-day web development conference, helping attendees discover talks, plan their agenda and get tickets.',
      pages: 'home with countdown timer, dates, venue and a "Get tickets" CTA; speakers grid with bio modals; schedule with Day 1 / Day 2 tabs and parallel tracks; ticket tiers page; venue and travel page; FAQ; sponsors section.',
      features: [
        'Schedule filters by track and level, plus search across talk titles and speakers',
        'Star talks to build a personal agenda; detect and flag time clashes',
        'Ticket selection with quantities and a demo checkout (no real payment) that issues a ticket code',
        'Add-to-calendar (.ics download) for any talk',
      ],
      data: 'about 30 sample talks and 16 sample speakers in a typed data file labelled as sample content; the personal agenda and demo tickets persist in localStorage.',
      style: 'energetic tech-event look: dark background, electric blue and violet gradients, bold geometric shapes, monospace accents for times, animated countdown.',
    }),
  },

  // ------------------------------------------------------------------ apps
  {
    id: 'productivity-dashboard',
    title: 'Productivity dashboard',
    description: 'Widgets, charts, tasks and notes in one customizable dashboard.',
    category: 'app',
    tags: ['Dashboard', 'Charts', 'Dark mode'],
    projectName: 'Productivity Hub',
    colors: ['#7c5cff', '#5aa9ff'],
    prompt: brief({
      goal: 'A personal productivity dashboard that brings tasks, notes, focus time and progress charts together on one screen, for people who want to plan and review their day.',
      pages: 'dashboard with a widget grid; tasks page; notes page; insights page with charts; settings page.',
      features: [
        'Widgets: greeting with date and weather placeholder, today\'s tasks, quick note, focus timer, weekly progress chart, upcoming deadlines',
        'Tasks with priorities, due dates, tags, subtasks, filters (today, upcoming, done) and drag-to-reorder',
        'Notes with markdown-style formatting, pinning and search',
        'Insights: completed tasks per day (bar chart), focus minutes (line chart) and tag breakdown (donut), drawn with lightweight SVG charts',
        'Show/hide and reorder widgets; dark mode toggle; keyboard shortcuts (N new task, / search)',
      ],
      data: 'everything persists in localStorage; on first launch the app offers to load clearly labelled sample data (with a "Clear sample data" button) so the charts are not empty.',
      style: 'calm, focused dark-first UI with a light theme: soft surfaces, violet accent, rounded cards, clear typographic hierarchy, smooth micro-interactions.',
    }),
  },
  {
    id: 'kanban',
    title: 'Kanban board',
    description: 'Drag-and-drop boards with labels, due dates and card details.',
    category: 'app',
    tags: ['Productivity', 'Drag & drop', 'Teams'],
    projectName: 'Kanban Board',
    colors: ['#3ecf8e', '#7c5cff'],
    prompt: brief({
      goal: 'A Trello-style kanban app for small teams and solo makers to organise work across multiple boards.',
      pages: 'boards overview; board view with columns; card detail drawer; archive view.',
      features: [
        'Create, rename, reorder and delete boards and columns',
        'Drag and drop cards between and within columns (mouse, touch and keyboard accessible)',
        'Card details: description, checklist with progress, labels with colours, due date, assignee avatar initials, comments',
        'Board filters (label, assignee, due soon, overdue) and quick search',
        'Column WIP limits with a warning when exceeded; undo for deletions',
      ],
      data: 'boards, columns and cards persist in localStorage; a first-run sample board ("Website relaunch - sample") demonstrates the features and can be deleted; JSON export/import of a board.',
      style: 'crisp and colourful: neutral board background, white (or dark) cards with subtle shadows, colourful label chips, satisfying drag animations.',
    }),
  },
  {
    id: 'finance-tracker',
    title: 'Finance tracker',
    description: 'Budgets, transactions and monthly charts to understand your money.',
    category: 'app',
    tags: ['Money', 'Budgets', 'Charts'],
    projectName: 'Finance Tracker',
    colors: ['#3ecf8e', '#f2b45a'],
    prompt: brief({
      goal: 'A personal finance tracker that helps people see where their money goes and stay within monthly budgets.',
      pages: 'overview dashboard; transactions list; budgets; reports; settings (currency, categories).',
      features: [
        'Add, edit and delete income and expense transactions with category, date, note and recurring option',
        'Monthly budgets per category with progress bars that turn amber and red as they fill',
        'Overview: balance, income vs expenses this month, spending by category (donut) and a 6-month trend (bar chart) using SVG charts',
        'Filters by month, category and type, plus search; CSV export and import',
        'Currency selector with proper number formatting',
      ],
      data: 'stored in localStorage; first launch offers three months of realistic sample transactions, clearly labelled as sample, that can be cleared in one click.',
      style: 'trustworthy and clear: clean light theme with a dark option, green for income, coral for expenses, large readable figures, tabular numbers.',
    }),
  },
  {
    id: 'habit-fitness',
    title: 'Habit & fitness tracker',
    description: 'Daily habits, streaks, workouts and progress rings.',
    category: 'app',
    tags: ['Health', 'Streaks', 'Charts'],
    projectName: 'Habit Tracker',
    colors: ['#ff7a59', '#ff7ab8'],
    prompt: brief({
      goal: 'A motivating habit and fitness tracker for people building healthy routines, inspired by activity rings and streak apps.',
      pages: 'today view; habits; workouts log; progress; settings.',
      features: [
        'Create habits with icon, colour, target (daily or x times per week) and reminders text',
        'Check off habits for today with a satisfying animation; streak counters and best streaks',
        'Workout log: type, duration, intensity and notes; weekly minutes goal shown as a progress ring',
        'Progress: calendar heatmap per habit, weekly completion chart and personal records',
        'Gentle encouragement messages and milestone badges (7, 30, 100 days)',
      ],
      data: 'saved in localStorage; optional sample history for the last 60 days (labelled as sample) so the charts and heatmap look alive; export/import as JSON.',
      style: 'energetic but clean: dark mode by default with vivid coral, pink and lime accents, big rounded rings, playful micro-animations, mobile-first layout with a bottom tab bar on phones.',
    }),
  },
  {
    id: 'ai-chat-ui',
    title: 'Chat-style AI app UI',
    description: 'A polished assistant chat with conversations, markdown and a mock model.',
    category: 'app',
    tags: ['Chat', 'AI', 'UI kit'],
    projectName: 'AI Chat',
    colors: ['#a48bff', '#5aa9ff'],
    prompt: brief({
      goal: 'A polished ChatGPT-style chat interface for a fictional assistant called "Nova", built as a front-end prototype with a simulated model so it works without any API key.',
      pages: 'chat view with sidebar of conversations; settings modal (model style, theme); empty state with suggested prompts.',
      features: [
        'Conversations: create, rename, pin, delete and search; grouped by Today / Previous 7 days / Older',
        'Messages render markdown (headings, lists, tables, code blocks with copy button)',
        'Simulated streaming replies from a local mock "model" with a typing indicator and a Stop button; clearly marked as a demo assistant',
        'Composer with auto-growing textarea, Enter to send / Shift+Enter for newline, regenerate and edit-last-message',
        'Keyboard shortcuts and a responsive layout with a collapsible sidebar',
      ],
      data: 'conversations and settings persist in localStorage; a couple of sample conversations, labelled as sample, show the formatting.',
      style: 'minimal and premium: neutral greys, subtle violet accent, comfortable message width, soft avatars, refined dark and light themes.',
    }),
  },
  {
    id: 'booking',
    title: 'Booking & appointments',
    description: 'Services, staff, available time slots and a booking manager.',
    category: 'app',
    tags: ['Scheduling', 'Calendar', 'Business'],
    projectName: 'Booking App',
    colors: ['#5aa9ff', '#3ecf8e'],
    prompt: brief({
      goal: 'An appointment booking app for "Bloom Studio", a fictional hair and beauty salon, with a customer booking flow and a simple staff view.',
      pages: 'services list; booking flow (service → staff → date & time → details → confirmation); my bookings; staff calendar (day and week views); settings for opening hours.',
      features: [
        'Available time slots computed from opening hours, service duration and existing bookings (no double booking)',
        'Reschedule and cancel bookings with confirmation dialogs',
        'Staff calendar with colour-coded appointments and a daily summary',
        'Booking confirmation with a reference code and add-to-calendar (.ics) download',
      ],
      data: 'sample services, prices, staff and a week of sample bookings (labelled as sample) are seeded on first launch; all bookings persist in localStorage.',
      style: 'soft and elegant: light background, sage and sky-blue accents, rounded pill time slots, clear step indicator, calm transitions.',
    }),
  },

  // ------------------------------------------------------------------ games
  {
    id: 'game-2048',
    title: '2048 puzzle',
    description: 'The classic sliding-tile puzzle with smooth animations and undo.',
    category: 'game',
    tags: ['Puzzle', 'Keyboard', 'Touch'],
    projectName: '2048',
    colors: ['#f2b45a', '#ff7a59'],
    prompt: brief({
      goal: 'A beautifully animated 2048 sliding-tile puzzle game that feels great on desktop and mobile.',
      pages: 'game screen with score, best score and New game; how-to-play modal; statistics panel.',
      features: [
        'Correct 2048 rules: slide, merge once per move, spawn a 2 (90%) or 4 (10%) after each valid move, detect win (2048) and game over',
        'Smooth tile slide, merge pop and spawn animations',
        'Arrow keys / WASD and swipe gestures; one-step undo',
        'Keep playing after 2048; board size option (4x4, 5x5)',
        'Statistics: games played, best tile, best score',
      ],
      data: 'best score, statistics and the in-progress game persist in localStorage so a refresh never loses a game.',
      style: 'warm and tactile: rounded tiles with a colour ramp from sand to orange to red, bold numbers, soft shadows, light and dark themes.',
      mustHaves: 'the game logic lives in pure functions with unit tests',
    }),
  },
  {
    id: 'game-snake',
    title: 'Neon snake',
    description: 'A retro arcade snake with levels, power-ups and a high-score table.',
    category: 'game',
    tags: ['Arcade', 'Canvas', 'Retro'],
    projectName: 'Neon Snake',
    colors: ['#3ecf8e', '#5aa9ff'],
    prompt: brief({
      goal: 'A neon-styled Snake arcade game with modern polish: levels, power-ups and local high scores.',
      pages: 'start screen; game screen (canvas) with HUD; pause overlay; game over screen with high-score entry; high scores table; settings.',
      features: [
        'Smooth canvas rendering with a fixed-timestep game loop; speed increases with level',
        'Controls: arrow keys/WASD, on-screen D-pad and swipe on touch devices; P to pause',
        'Power-ups: slow motion, double points and shrink, each with a timed indicator',
        'Optional wall wrap-around mode and obstacle levels',
        'Sound effects via the Web Audio API with a mute toggle',
      ],
      data: 'top 10 high scores (name, score, level, date) and settings are saved in localStorage; the table starts with a few sample scores labelled as sample.',
      style: 'retro-futuristic: near-black background, glowing neon green and cyan lines, subtle grid and scanline effect, pixel-style score font.',
      mustHaves: 'the game state update is a pure function with unit tests',
    }),
  },
  {
    id: 'game-memory',
    title: 'Memory match',
    description: 'A card-flipping memory game with themes, timer and difficulty levels.',
    category: 'game',
    tags: ['Casual', 'Kids', 'Animations'],
    projectName: 'Memory Match',
    colors: ['#ff7ab8', '#f2b45a'],
    prompt: brief({
      goal: 'A delightful card-matching memory game suitable for kids and adults.',
      pages: 'home with theme and difficulty selection; game board; results screen with stars; best times.',
      features: [
        'Difficulty levels: 4x3, 4x4 and 6x5 grids; shuffled fairly every game',
        'Card themes (animals, space, food) drawn with emoji or inline SVG',
        '3D card flip animation; matched pairs celebrate; mismatches flip back after a short delay',
        'Move counter, timer and a 1-3 star rating; confetti on completion',
        'Two-player hot-seat mode with turn indicator and scores',
      ],
      data: 'best times and fewest moves per difficulty are stored in localStorage.',
      style: 'playful and bright: pastel gradients, chunky rounded cards, friendly rounded font, bouncy but short animations.',
    }),
  },

  // ------------------------------------------------------------------ tools
  {
    id: 'docs-knowledge-base',
    title: 'Docs / knowledge base',
    description: 'A searchable documentation site with sidebar navigation and editor.',
    category: 'tool',
    tags: ['Docs', 'Search', 'Markdown'],
    projectName: 'Knowledge Base',
    colors: ['#5aa9ff', '#a48bff'],
    prompt: brief({
      goal: 'A documentation and knowledge-base app for a fictional product "Acme Cloud", usable as a public docs site and as an internal wiki.',
      pages: 'docs layout with collapsible sidebar tree, article view and an "On this page" outline; search results; article editor; recently updated page.',
      features: [
        'Instant search (Ctrl+K) across titles and content with highlighted snippets',
        'Articles written in markdown with headings, code blocks (copy button), callouts and tables',
        'Create, edit (live preview), reorder and delete articles and sections',
        'Breadcrumbs, previous/next links and "Was this helpful?" feedback',
      ],
      data: 'around 15 sample articles (getting started, guides, API reference, FAQ), labelled as sample content, are seeded on first launch; edits, new articles and feedback persist in localStorage; export everything as JSON.',
      style: 'clean developer-docs look: white or dark surfaces, blue accent, monospace for code, readable 16px body text, sticky header with search.',
    }),
  },
  {
    id: 'invoice-generator',
    title: 'Invoice generator',
    description: 'Create, preview and print professional invoices for freelancers.',
    category: 'tool',
    tags: ['Business', 'Print', 'PDF'],
    projectName: 'Invoices',
    colors: ['#7c5cff', '#3ecf8e'],
    prompt: brief({
      goal: 'An invoice generator for freelancers and small studios to create professional invoices in minutes and track whether they are paid.',
      pages: 'invoices list with status filters; invoice editor with live A4 preview side by side; clients; business profile settings.',
      features: [
        'Line items with quantity, rate, discount and tax; automatic subtotal, tax and total',
        'Invoice numbering, issue and due dates, currency and payment terms',
        'Statuses (draft, sent, paid, overdue) with a dashboard summary of outstanding amounts',
        'Print-perfect layout with a print stylesheet so "Print / Save as PDF" looks professional',
        'Duplicate invoices and reuse saved clients',
      ],
      data: 'business profile, clients and invoices persist in localStorage; three sample invoices and clients, labelled as sample, demonstrate each status.',
      style: 'professional and minimal: lots of whitespace, violet accent, elegant tabular figures, a document preview that looks like real paper.',
    }),
  },
  {
    id: 'pomodoro',
    title: 'Pomodoro focus timer',
    description: 'A focus timer with sessions, task list, ambient sounds and stats.',
    category: 'tool',
    tags: ['Focus', 'Timer', 'Stats'],
    projectName: 'Focus Timer',
    colors: ['#ff7a59', '#7c5cff'],
    prompt: brief({
      goal: 'A beautiful Pomodoro focus timer that helps people work in focused sessions and see their progress over time.',
      pages: 'timer view; tasks panel; statistics; settings.',
      features: [
        'Focus, short break and long break modes with configurable durations and auto-start options',
        'Large animated progress ring; remaining time also shown in the browser tab title',
        'Task list: pick the current task, estimate pomodoros, mark done',
        'Gentle end-of-session chime and optional ambient noise generated with the Web Audio API',
        'Statistics: focus minutes per day (bar chart) and streaks',
      ],
      data: 'settings, tasks and session history persist in localStorage; sample history (labelled as sample) can be loaded to preview the statistics.',
      style: 'calm and focused: the background colour shifts gently between modes (warm coral for focus, teal for breaks), large thin numerals, minimal chrome.',
    }),
  },
];

/** Blueprints in a category ("all" for every one), optionally matching a search query. */
export function filterBlueprints(list: Blueprint[], filter: BlueprintFilter, query = ''): Blueprint[] {
  const q = query.trim().toLowerCase();
  return list.filter(
    (b) =>
      (filter === 'all' || b.category === filter) &&
      (!q || [b.title, b.description, ...b.tags].some((s) => s.toLowerCase().includes(q))),
  );
}

export function blueprintById(id: string): Blueprint | undefined {
  return BLUEPRINTS.find((b) => b.id === id);
}
