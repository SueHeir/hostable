// Noughts and crosses, written against @hostable/sdk. Builds to one
// rules.js that runs in Hostable's QuickJS sandbox.

import { defineGame, ui, type Outcome, type Seat, type TableView, type Widget, type Shape } from "@hostable/sdk";

type Mark = "X" | "O" | null;

interface State {
  names: string[];
  board: Mark[];
  turn: 0 | 1;
  winner: 0 | 1 | null;
  draw: boolean;
  line: number[] | null;
}

const LINES = [
  [0, 1, 2], [3, 4, 5], [6, 7, 8],
  [0, 3, 6], [1, 4, 7], [2, 5, 8],
  [0, 4, 8], [2, 4, 6],
];
const MARKS: ["X", "O"] = ["X", "O"];
const TURN_LIVE_MS = 30_000;
const TURN_ASYNC_MS = 24 * 3600_000;

function waiting(s: State): Outcome<State> {
  if (s.winner !== null || s.draw) {
    const ranking = s.draw ? [[0, 1]] : [[s.winner as number], [1 - (s.winner as number)]];
    const summary = s.draw ? "A draw." : `${s.names[s.winner as number]} wins!`;
    return { state: s, finished: { ranking, summary }, events: [{ type: "sound", name: "fanfare" }] };
  }
  return {
    state: s,
    waiting_on: [{ seat: s.turn, prompt: "Your move", live_timeout_ms: TURN_LIVE_MS, async_timeout_ms: TURN_ASYNC_MS }],
  };
}

function place(s: State, cell: number): State {
  const board = s.board.slice();
  board[cell] = MARKS[s.turn];
  const line = LINES.find((l) => l.every((i) => board[i] === MARKS[s.turn])) ?? null;
  const winner = line ? s.turn : null;
  const draw = !line && board.every((m) => m !== null);
  return { ...s, board, line, winner, draw, turn: (1 - s.turn) as 0 | 1 };
}

export const game = defineGame<State>({
  seats: { min: 2, max: 2 },

  setup(seats: Seat[]) {
    return waiting({
      names: seats.map((s) => s.name),
      board: Array(9).fill(null),
      turn: 0,
      winner: null,
      draw: false,
      line: null,
    });
  },

  onAction(s, seat, input) {
    const cell = Number(input.widget.replace("cell-", ""));
    const playable = seat === s.turn && s.winner === null && !s.draw &&
      input.widget.startsWith("cell-") && Number.isInteger(cell) && cell >= 0 && cell < 9 && s.board[cell] === null;
    if (!playable) return { state: s, events: [{ type: "feedback", seat, kind: "warning" }] };
    return waiting(place(s, cell));
  },

  // A player who runs out of time gets the first free square.
  onTimeout(s, seat) {
    if (seat !== s.turn || s.winner !== null || s.draw) return waiting(s);
    const cell = s.board.findIndex((m) => m === null);
    return waiting(place(s, cell));
  },

  tableView(s): TableView {
    const shapes: Shape[] = [];
    for (let i = 0; i < 9; i++) {
      const x = 20 + (i % 3) * 100;
      const y = 20 + Math.floor(i / 3) * 100;
      const inLine = s.line?.includes(i) ?? false;
      shapes.push({ kind: "rect", x, y, w: 90, h: 90, fill: inLine ? "#f3a712" : "#fdfdf8", stroke: "#29335c", label: s.board[i] ?? "" });
    }
    const status = s.winner !== null
      ? `${s.names[s.winner]} wins!`
      : s.draw ? "A draw." : `${s.names[s.turn]} to play (${MARKS[s.turn]})`;
    return {
      title: "Noughts and crosses",
      status,
      scene: { width: 330, height: 330, background: "#29335c", shapes },
      players: s.names.map((name, i) => ({ name, color: i === 0 ? "#e4572e" : "#669bbc", lines: [MARKS[i]], active: s.turn === i && s.winner === null && !s.draw })),
      log: [],
    };
  },

  phoneView(s, seat): Widget {
    const mine = MARKS[seat];
    const myTurn = seat === s.turn && s.winner === null && !s.draw;
    const cells = s.board.map((m, i) => ui.button(`cell-${i}`, m ?? " ", "huge", !myTurn || m !== null));
    let header: string;
    if (s.winner !== null) header = s.winner === seat ? "You win!" : `${s.names[s.winner]} wins`;
    else if (s.draw) header = "A draw.";
    else header = myTurn ? `Your move (${mine})` : `Waiting for ${s.names[s.turn]}`;
    return ui.column(ui.title(header), ui.grid(3, cells));
  },

  canRespond(s, seat) {
    return seat === s.turn && s.winner === null && !s.draw;
  },
});
