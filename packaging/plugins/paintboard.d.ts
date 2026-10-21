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
    /** Where its link goes: a web or mail address, or the id of an object on the board. */
    link: string | null;
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
    /** Tables only: the text of each cell, row by row (merged cells show their text in
     * the top left one). */
    cells?: string[][];
    /** Stickies, shapes and text objects with lists: each paragraph (the text between line
     * breaks), with its list kind and nesting level. Read only. */
    paragraphs?: { text: string; list: "bullet" | "number" | null; level: number }[];
    /** Stickies, shapes and text objects with links in their text: the linked text and
     * where it goes, in order. Read only. */
    links?: { text: string; link: string }[];
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
    /** Names of tags to put on it; tags the board does not have yet are added. */
    tags?: string[];
    /** A web or mail address, or the id of an object to go to (a frame, say). */
    link?: string;
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
        /** A label in the middle of the line. */
        text?: string;
      })
    | (NewBase & {
        type: "table";
        /** The cells' text, row by row; shorter rows are filled with empty cells. Up to
         * 500 rows and 50 columns. */
        rows: string[][];
        /** Every column this wide; without it each fits its text. */
        columnWidth?: number;
        /** Every row this high; without it each fits its text. */
        rowHeight?: number;
      });

  /** What `update` can change. */
  interface Changes {
    x?: number;
    y?: number;
    width?: number;
    height?: number;
    rotation?: number;
    /** Stickies, shapes and text objects; a frame's title; a connector's label. */
    text?: string;
    title?: string;
    fill?: Color;
    stroke?: Color;
    strokeWidth?: number;
    opacity?: number;
    /** The names of all the tags it should carry (others are taken off); tags the board
     * does not have yet are added. */
    tags?: string[];
    /** A web or mail address, or an object's id; `null` removes the link. */
    link?: string | null;
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
    /** Adds a tag to the board, or gives the tag with that name (in any case) the color;
     * resolves to its id. Without a color a new tag gets the next of the usual ones. */
    function defineTag(tag: { name: string; color?: Color }): Promise<string>;
    /** Creates objects in one step; resolves to their ids. */
    function create(items: NewObject | NewObject[]): Promise<Id[]>;
    function update(id: Id, changes: Changes): Promise<void>;
    function remove(ids: Id | Id[]): Promise<void>;
    /** Puts `text` in a table's cell; `row` and `col` count from 0. */
    function setCell(id: Id, row: number, col: number, text: string): Promise<void>;
    /** Groups two or more objects (or groups); resolves to the new group's id. */
    function group(ids: Id[]): Promise<Id>;
    /** Dissolves groups; an object's id stands for the group directly containing it. */
    function ungroup(ids: Id | Id[]): Promise<void>;
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
    /** Plays one of PaintBoard's short sounds. Needs no permission. */
    function playSound(name: "chime" | "beep" | "done" | "alarm"): Promise<void>;
    /** Resolves after `ms` milliseconds (up to a day). `setTimeout` and `clearTimeout`
     * work too, on the desktop through this. */
    function after(ms: number): Promise<void>;
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
