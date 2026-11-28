/** Sends `{"type":"input","widget":id,"value":value}` for a widget. */
export type SendFn = (widget: string, value: unknown) => void;
