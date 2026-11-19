window.__buhoHost = true;
window.__installBuhoControl = function installBuhoControl() {
  if (window.__buhoControl) return;

  function clean(text) {
    return String(text || "")
      .replace(/\s+/g, " ")
      .replace(/\s*\*\s*$/, "")
      .trim();
  }

  function norm(text) {
    return clean(text).toLowerCase();
  }

  function cssEscape(value) {
    if (window.CSS && typeof CSS.escape === "function") return CSS.escape(value);
    return String(value).replace(/(["\\])/g, "\\$1");
  }

  function labelFor(el) {
    if (el.id) {
      const label = document.querySelector('label[for="' + cssEscape(el.id) + '"]');
      if (label) return clean(label.textContent);
    }
    const wrapped = el.closest("label");
    if (wrapped) return clean(wrapped.textContent);
    return clean(el.getAttribute("aria-label") || "");
  }

  function controls(root) {
    return Array.from(root.querySelectorAll("input, textarea, select")).filter(function (el) {
      const type = (el.getAttribute("type") || "").toLowerCase();
      return type !== "hidden" && type !== "submit" && type !== "button" && type !== "file";
    });
  }

  function readField(el) {
    const type = (el.getAttribute("type") || el.tagName).toLowerCase();
    let value = el.value;
    if (type === "checkbox" || type === "radio") value = el.checked ? el.value || true : false;
    return {
      name: el.name || el.id || labelFor(el),
      label: labelFor(el),
      type: type,
      value: value,
      required: Boolean(el.required),
      disabled: Boolean(el.disabled),
      placeholder: el.placeholder || "",
    };
  }

  function buttonText(button) {
    return clean(button.innerText || button.value || button.getAttribute("aria-label") || "");
  }

  function formRoots() {
    const forms = Array.from(document.forms);
    return forms.length ? forms : [document.body];
  }

  function describeRoot(root, index) {
    return {
      id: root.id || "",
      name: root.getAttribute("name") || "",
      index: index,
      fields: controls(root).map(readField),
      buttons: Array.from(root.querySelectorAll("button, input[type=submit]"))
        .map(buttonText)
        .filter(Boolean),
    };
  }

  function pagesFrom(selector, limit) {
    const pages = [];
    const seen = {};
    Array.from(document.querySelectorAll(selector)).forEach(function (anchor) {
      if (pages.length >= limit) return;
      if (!anchor.href) return;
      try {
        const url = new URL(anchor.href, location.href);
        const path = url.pathname + url.search;
        if (seen[path]) return;
        seen[path] = true;
        pages.push({ name: clean(anchor.textContent) || path, path: path, url: url.href });
      } catch (error) {}
    });
    return pages;
  }

  function setNativeValue(el, value) {
    const prototype = Object.getPrototypeOf(el);
    const descriptor = Object.getOwnPropertyDescriptor(prototype, "value");
    if (descriptor && descriptor.set) descriptor.set.call(el, value);
    else el.value = value;
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  }

  function visible(el) {
    const style = window.getComputedStyle(el);
    return style.display !== "none" && style.visibility !== "hidden" && el.getClientRects().length > 0;
  }

  function query(selector) {
    const key = String(selector || "").trim();
    if (!key) throw new Error("selector is required.");
    const byCss = document.querySelector(key);
    if (byCss) return byCss;
    const wanted = norm(key);
    const nodes = Array.from(
      document.querySelectorAll("button, a, input, textarea, select, [role=button], [aria-label]"),
    ).filter(visible);
    const match = nodes.find(function (el) {
      return (
        norm(el.id) === wanted ||
        norm(el.name || "") === wanted ||
        norm(labelFor(el)) === wanted ||
        norm(buttonText(el)) === wanted ||
        norm(el.getAttribute("placeholder") || "") === wanted
      );
    });
    if (!match) throw new Error("No element matches " + key + ".");
    return match;
  }

  const builtins = {
    "browser.snapshot": function () {
      const roots = formRoots();
      return {
        url: location.href,
        path: location.pathname + location.search + location.hash,
        title: document.title,
        headings: Array.from(document.querySelectorAll("h1, h2, h3"))
          .map(function (node) {
            return clean(node.textContent);
          })
          .filter(Boolean)
          .slice(0, 20),
        forms: roots.map(describeRoot),
        links: pagesFrom("a[href]", 40),
      };
    },
    "browser.navigate": function (args) {
      const href = String((args && (args.url || args.path)) || "").trim();
      if (!href) throw new Error("url is required.");
      const url = new URL(href, location.href);
      if (url.protocol !== "http:" && url.protocol !== "https:") {
        throw new Error("The page URL must start with http:// or https://.");
      }
      return { url: url.href, navigateTo: url.href };
    },
    "browser.click": function (args) {
      const el = query((args && (args.selector || args.text)) || "");
      el.click();
      return { clicked: true, text: buttonText(el) || clean(el.textContent) };
    },
    "browser.type": function (args) {
      const el = query((args && args.selector) || "");
      const text = args && args.text != null ? String(args.text) : "";
      el.focus();
      if ("value" in el) setNativeValue(el, String(el.value || "") + text);
      else {
        el.dispatchEvent(new InputEvent("beforeinput", { bubbles: true, data: text }));
        el.dispatchEvent(new Event("input", { bubbles: true }));
      }
      return { typed: true, text: text };
    },
    "browser.fill": function (args) {
      const el = query((args && (args.selector || args.label)) || "");
      const value = args && args.value != null ? args.value : "";
      if (el.tagName === "SELECT") {
        const wanted = String(value);
        const option = Array.from(el.options).find(function (item) {
          return item.value === wanted || clean(item.textContent) === wanted;
        });
        if (!option) throw new Error("No option matches " + wanted + ".");
        setNativeValue(el, option.value);
      } else if ((el.getAttribute("type") || "").toLowerCase() === "checkbox") {
        el.checked = value === true || value === "true" || value === "on" || value === 1 || value === "1";
        el.dispatchEvent(new Event("input", { bubbles: true }));
        el.dispatchEvent(new Event("change", { bubbles: true }));
      } else {
        setNativeValue(el, value == null ? "" : String(value));
      }
      return { filled: true, selector: args.selector || args.label || "" };
    },
  };

  window.__buhoControl = {
    call: async function (name, args) {
      const builtin = builtins[name];
      if (typeof builtin !== "function") throw new Error("Unknown tool " + name + ".");
      return await builtin(args || {});
    },
  };
};

window.__installBuhoControl();
