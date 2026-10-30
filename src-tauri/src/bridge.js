window.__installAppControl = function installAppControl() {
  if (window.__appControl) return;

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

  function findRoot(formKey) {
    const roots = formRoots();
    if (formKey == null || String(formKey).trim() === "") {
      if (roots.length === 1) return roots[0];
      throw new Error("This page has more than one form. Pass form.");
    }
    const key = String(formKey);
    const byId = document.getElementById(key);
    if (byId && (byId.tagName === "FORM" || byId === document.body)) return byId;
    const named = roots.find(function (form) {
      return form.name === key || form.id === key || form.getAttribute("aria-label") === key;
    });
    if (named) return named;
    const index = Number(key);
    if (String(index) === key && roots[index]) return roots[index];
    throw new Error("No form matches " + key + ".");
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

  function sameOrigin(href) {
    try {
      const url = new URL(href, location.href);
      if (url.origin !== location.origin) return null;
      return url;
    } catch (error) {
      return null;
    }
  }

  function pagesFrom(selector, limit) {
    const pages = [];
    const seen = {};
    Array.from(document.querySelectorAll(selector)).forEach(function (anchor) {
      if (pages.length >= limit) return;
      const url = sameOrigin(anchor.href);
      if (!url) return;
      const path = url.pathname + url.search;
      if (seen[path]) return;
      seen[path] = true;
      pages.push({ name: clean(anchor.textContent) || path, path: path });
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

  function findControl(root, key) {
    const all = controls(root).filter(function (el) {
      return !el.disabled;
    });
    const exact = all.find(function (el) {
      return el.name === key || el.id === key;
    });
    if (exact) return exact;
    const wanted = norm(key);
    return (
      all.find(function (el) {
        return norm(labelFor(el)) === wanted || norm(el.getAttribute("placeholder") || "") === wanted;
      }) || null
    );
  }

  function setControl(root, el, value) {
    const type = (el.getAttribute("type") || "").toLowerCase();
    if (type === "checkbox") {
      const on = value === true || value === "true" || value === "on" || value === 1 || value === "1";
      el.checked = on;
      el.dispatchEvent(new Event("input", { bubbles: true }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
      return;
    }
    if (type === "radio") {
      const group = Array.from(
        root.querySelectorAll('input[type="radio"][name="' + cssEscape(el.name) + '"]'),
      );
      const match = group.find(function (radio) {
        return radio.value === String(value);
      });
      if (!match) throw new Error("No choice matches " + String(value) + ".");
      match.checked = true;
      match.dispatchEvent(new Event("input", { bubbles: true }));
      match.dispatchEvent(new Event("change", { bubbles: true }));
      return;
    }
    if (el.tagName === "SELECT") {
      const wanted = String(value);
      const option = Array.from(el.options).find(function (item) {
        return item.value === wanted || clean(item.textContent) === wanted;
      });
      if (!option) throw new Error("No option matches " + wanted + ".");
      setNativeValue(el, option.value);
      return;
    }
    setNativeValue(el, value == null ? "" : String(value));
  }

  const builtins = {
    list_pages: function () {
      const pages = pagesFrom("nav a[href], header a[href], [role=navigation] a[href]", 80);
      return { pages: pages };
    },
    describe_page: function () {
      const roots = formRoots();
      return {
        url: location.href,
        path: location.pathname + location.search + location.hash,
        title: document.title,
        forms: roots.map(describeRoot),
        links: pagesFrom("a[href]", 40),
      };
    },
    navigate: function (args) {
      const path = String((args && args.path) || "").trim();
      if (!path) throw new Error("path is required.");
      const url = new URL(path, location.href);
      if (url.origin !== location.origin) throw new Error("That path is outside this app.");
      return {
        path: url.pathname + url.search + url.hash,
        navigateTo: url.href,
      };
    },
    set_fields: function (args) {
      const fields = args && args.fields;
      if (!fields || typeof fields !== "object" || Array.isArray(fields)) {
        throw new Error("fields must be an object of field names to values.");
      }
      const keys = Object.keys(fields);
      if (!keys.length) throw new Error("fields must include at least one field.");
      const root = findRoot(args.form);
      const set = [];
      const missing = [];
      keys.forEach(function (key) {
        const el = findControl(root, key);
        if (!el) {
          missing.push(key);
          return;
        }
        setControl(root, el, fields[key]);
        set.push(key);
      });
      if (!set.length) throw new Error("None of those fields are on this form.");
      return { set: set, missing: missing, form: describeRoot(root, 0) };
    },
    submit_form: function (args) {
      const root = findRoot(args && args.form);
      const buttons = Array.from(root.querySelectorAll("button, input[type=submit]")).filter(function (button) {
        return !button.disabled;
      });
      let button = null;
      if (args && args.button) {
        const wanted = norm(args.button);
        button = buttons.find(function (item) {
          return norm(buttonText(item)) === wanted;
        });
        if (!button) throw new Error("No button matches " + args.button + ".");
      } else {
        button =
          buttons.find(function (item) {
            return (item.getAttribute("type") || "").toLowerCase() === "submit";
          }) ||
          buttons.find(function (item) {
            return /save|submit|create|update|log in|sign in/i.test(buttonText(item));
          }) ||
          null;
      }
      if (button) {
        button.click();
        return { submitted: true, button: buttonText(button) };
      }
      if (typeof root.requestSubmit === "function") {
        root.requestSubmit();
        return { submitted: true, button: "" };
      }
      throw new Error("This page has no submit button.");
    },
  };

  window.__appControl = {
    builtins: builtins,
    call: async function (name, args) {
      const hooks = window.__appControlHooks;
      const hook = hooks && hooks[name];
      if (typeof hook === "function") return await hook(args || {});
      const builtin = builtins[name];
      if (typeof builtin !== "function") throw new Error("Unknown tool " + name + ".");
      return await builtin(args || {});
    },
  };
};

window.__installAppControl();
