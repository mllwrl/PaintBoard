// Types of the `paintboard` object PaintBoard gives plugins, API version 1.
// Copy this file next to your plugin's sources to get completion and checks in your
// editor; PaintBoard itself runs plain JavaScript (compile TypeScript with tsc or esbuild).

declare namespace paintboard {
  /** The plugin API version of the app running the plugin. */
  const apiVersion: 1;

  /** An id of an object on the board, like "12@3456". */
  type Id = string;

  /** A color: "#rrggbb" or "#rrggbbaa". */
  type Color = string;

  type ObjectType =
    | "shape" | "sticky" | "text" | "frame" | "connector" | "ink" | "image"
    | "table" | "kanban" | "mindmap" | "doc" | "embed";

  /** Kinds of shapes, as `shape` in a shape object. */
  type ShapeKind =
    | "rect" | "roundRect" | "ellipse" | "diamond" | "triangle" | "star" | "arrow"
    | "doubleArrow" | "cloud" | string;

  /** An object as plugins see it. Positions are the unrotated top left corner in board
   * units; rotation is in degrees, clockwise. */
  interface BoardObject {
    id: Id;
    type: ObjectType;
    x: number;
    y: number;
    width: number;
    height: number;
    rotation: number;
    fill: Color;
    stroke: Color;
    strokeWidth: number;
    opacity: number;
    /** Names of the object's tags. */
    tags: string[];
    locked: boolean;
    /** The group directly containing it. */
    parent: Id | null;
    /** Text of stickies, shapes, text objects and documents; a frame's title; a
     * connector's label; a web card's title. */
    text?: string;
    /** Shapes only. */
    shape?: ShapeKind;
    /** Text objects only. */
    fontSize?: number;
    /** Frames only. */
    title?: string;
    /** Connectors only: the objects their ends are attached to. */
    from?: Id | null;
    to?: Id | null;
    /** Web cards only. */
    url?: string;
  }

  /** Which objects `getObjects` returns; all of them without a filter. */
  interface Filter {
    type?: ObjectType;
    /** The name of a tag they carry. */
    tag?: string;
    /** Only those inside this frame. */
    frame?: Id;
  }

  /** Common to new objects. Without x and y they go in the middle of the view. */
  interface NewBase {
    x?: number;
    y?: number;
    width?: number;
    height?: number;
    rotation?: number;
    fill?: Color;
    stroke?: Color;
    strokeWidth?: number;
    opacity?: number;
  }

  type NewObject =
    | (NewBase & { type?: "sticky"; text?: string })
    | (NewBase & { type: "shape"; shape?: ShapeKind; text?: string })
    | (NewBase & { type: "text"; text: string; fontSize?: number })
    | (NewBase & { type: "frame"; title?: string })
    | (NewBase & {
        type: "connector";
        from: Id | { x: number; y: number };
        to: Id | { x: number; y: number };
        route?: "straight" | "elbow" | "curved";
        /** An arrowhead at the end; true by default. */
        arrow?: boolean;
      });

  /** What `update` can change. */
  interface Changes {
    x?: number;
    y?: number;
    width?: number;
    height?: number;
    rotation?: number;
    /** Stickies, shapes and text objects; a frame's title. */
    text?: string;
    title?: string;
    fill?: Color;
    stroke?: Color;
    strokeWidth?: number;
    opacity?: number;
  }

  interface Viewport {
    x: number;
    y: number;
    width: number;
    height: number;
    zoom: number;
  }

  interface Tag {
    id: string;
    name: string;
    color: Color;
  }

  /** Reading needs the "board:read" permission, changing "board:write". Everything a
   * plugin changes while handling one command or panel event is one step for Ctrl+Z. */
  namespace board {
    function getObjects(filter?: Filter): Promise<BoardObject[]>;
    function getObject(id: Id): Promise<BoardObject | null>;
    function getSelection(): Promise<Id[]>;
    function select(ids: Id | Id[]): Promise<void>;
    function getViewport(): Promise<Viewport>;
    function zoomTo(ids: Id | Id[]): Promise<void>;
    function getTags(): Promise<Tag[]>;
    /** Creates objects in one step; resolves to their ids. */
    function create(items: NewObject | NewObject[]): Promise<Id[]>;
    function update(id: Id, changes: Changes): Promise<void>;
    function remove(ids: Id | Id[]): Promise<void>;
  }

  interface Command {
    id: string;
    title: string;
    /** Also offered in the right-click menu of selected objects. */
    selection?: boolean;
  }

  interface CommandEvent {
    id: string;
    /** What was selected when the command was chosen. */
    selection: Id[];
  }

  type PanelItem =
    | { type: "text"; text: string }
    | { type: "separator" }
    | { type: "input"; id: string; label?: string; value?: string; placeholder?: string; multiline?: boolean }
    | { type: "button"; id: string; label: string }
    | { type: "list"; id: string; label?: string; items: string[] };

  interface Panel {
    title?: string;
    items: PanelItem[];
  }

  /** A button pressed or a list item picked, with what the inputs hold. */
  interface PanelEvent {
    button?: string;
    list?: string;
    index?: number;
    values: Record<string, string>;
  }

  namespace ui {
    /** Adds a command to the Plugins menu. */
    function addCommand(command: Command, handler: (event: CommandEvent) => void | Promise<void>): Promise<void>;
    /** Shows the plugin's panel (one per plugin), replacing the one shown. */
    function showPanel(panel: Panel, handler?: (event: PanelEvent) => void | Promise<void>): Promise<void>;
    function closePanel(): Promise<void>;
    /** A short message at the bottom of the window. */
    function notify(text: string): Promise<void>;
  }

  /** Values kept for the plugin between runs, up to 1 MB in all. */
  namespace storage {
    function get(key: string): Promise<unknown>;
    /** `null` or `undefined` removes the key. */
    function set(key: string, value: unknown): Promise<void>;
  }

  interface FetchOptions {
    method?: "GET" | "POST" | "PUT" | "DELETE";
    /** Sent as JSON. */
    body?: string;
  }

  interface FetchAnswer {
    ok: boolean;
    status: number;
    text: string;
  }

  namespace net {
    /** Needs "network:<host>" for the address's host (or a domain it is under). */
    function fetch(url: string, options?: FetchOptions): Promise<FetchAnswer>;
  }

  namespace clipboard {
    /** Needs the "clipboard" permission. */
    function writeText(text: string): Promise<void>;
  }

  /** "selectionchange" gets the selected ids; "boardchange" comes after others' or the
   * user's changes (not the plugin's own). Both need "board:read". */
  function on(event: "selectionchange", handler: (ids: Id[]) => void): void;
  function on(event: "boardchange", handler: () => void): void;
}
