// NeoShard plugin sandbox loader. Runs inside an opaque-origin iframe with no
// network access. The host sends the (hash-verified) plugin code once; the
// plugin gets a tiny API object and nothing else.
(function () {
  "use strict";
  var listeners = [];
  var loaded = false;
  var api = Object.freeze({
    onMessage: function (cb) {
      if (typeof cb === "function") listeners.push(cb);
    },
    sendMessage: function (channelId, content) {
      parent.postMessage({ type: "send-message", channelId: String(channelId), content: String(content) }, "*");
    },
  });
  window.addEventListener("message", function (e) {
    if (e.source !== parent || !e.data || typeof e.data !== "object") return;
    if (e.data.type === "load" && !loaded && typeof e.data.code === "string") {
      loaded = true;
      try {
        new Function("neoshard", e.data.code)(api);
      } catch (err) {
        parent.postMessage({ type: "error", message: String(err) }, "*");
      }
    } else if (e.data.type === "message") {
      for (var i = 0; i < listeners.length; i++) {
        try {
          listeners[i](e.data);
        } catch (err) {
          /* plugin errors stay inside the sandbox */
        }
      }
    }
  });
  parent.postMessage({ type: "ready" }, "*");
})();
