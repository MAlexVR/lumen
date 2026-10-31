// Looping is done here, not with the <video loop> attribute: that one restarts
// through a segment seek in WebKitGTK that can cut the clip short. Waiting for
// the real `ended` event lets the video play to its last frame first.

export type AfterEnd = "replay" | "advance";

export const afterEnd = (loop: boolean): AfterEnd => (loop ? "replay" : "advance");
