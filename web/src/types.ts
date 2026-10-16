export interface Point { date: string; equity: number; benchmark: number | null; lqdt: number | null; mcftr?: number | null }
export interface Holding { ticker: string; side: string; weight: number; reason: string; until: string }
export interface Trade { ticker: string; side: string; entry_date: string; exit_date: string | null; entry_price: number; exit_price: number | null; quantity: number; pnl: number; reason: string }
export interface Run {
  label: string; mode: string; status: string; message: string | null; start: string | null; end: string | null;
  initial: number; final_equity: number; total_return: number; cagr: number; max_drawdown: number;
  parameters: string; tested: number; points: Point[]; trades: Trade[]; holdings: Holding[];
  selections: { effective: string; trained_until: string; parameters: string }[];
  warnings: string[]; leaderboard: { parameters: string; total_return: number; events: number }[];
  next_action?: string;
}
export interface Settings {
  capital: number; refresh: boolean; periods: number[]; entry_windows: number[]; exit_windows: number[];
  slots: number[]; training_start: string; start: string; min_events: number; fund: string;
}
export interface Report { id: number; calculated_at: string; settings: Settings; runs: Run[]; warnings: string[] }
export interface Snapshot {
  data: { updated_at: string | null; instruments: number; bars: number; dividends: number; verified_dividends: number; warnings: string[] };
  reports: Record<string, Report>; benchmarks: Record<string, Run>; defaults: Settings; today: string;
  job: null | { status: string; progress: string; strategy: number | null; error: string | null; started_at: string; finished_at: string | null };
}
export interface Sources {
  instruments: { ticker: string; name: string; kind: string; source: string; count: number; from: string; to: string; multiplier: number | null; expiry: string | null }[];
  dividends: { ticker: string; record_date: string; amount: number; declared_on: string | null; last_buy_date: string | null; source: string; announcement_source: string | null }[];
  warnings: string[];
}
