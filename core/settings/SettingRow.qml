import QtQuick
import QtQuick.Layouts
import ".."
import "Settings.js" as Settings

// One field of the schema: title and help on the left, the control on the
// right. Controls that need room (cards, module lists) go under the text
// instead. A `note` has no control at all, nor a key.
//
// Every control takes `field` and `value` (the effective value: config or
// default) and emits `edited(value)`; `undefined` removes the key, and so
// does the field's default.
ColumnLayout {
  id: row

  property var field: ({})
  property var svc: null

  readonly property var stored: row.svc ? row.svc.configValue(row.field.key) : undefined
  readonly property var value: row.stored === undefined ? row.field["default"] : row.stored
  readonly property bool wide: row.field.type === "modules" || row.field.type === "workspaces"
    || row.field.type === "notches" || row.field.type === "moduleCatalog" || row.field.type === "component"
    || (row.field.type === "choice" && (row.field.display === "cards" || row.field.display === "chips"))

  readonly property Component control: {
    switch (row.field.type) {
      case "choice": return row.field.display === "cards" ? cardsControl
                          : row.field.display === "chips" ? chipsControl : segmentedControl
      case "integer": return sliderControl
      case "number": return sliderControl
      case "boolean": return toggleControl
      case "string": return textControl
      case "color": return textControl
      case "modules": return modulesControl
      case "workspaces": return workspacesControl
      case "notches": return notchesControl
      case "moduleCatalog": return moduleCatalogControl
      case "component": return componentControl
      case "note": return null
      default: return unknownControl
    }
  }

  function edit(v) { if (row.svc) row.svc.setConfigValue(row.field.key, Settings.storedValue(row.field, v)) }

  spacing: Theme.space(12)

  RowLayout {
    Layout.fillWidth: true
    spacing: Theme.space(24)

    ColumnLayout {
      Layout.fillWidth: true
      Layout.alignment: Qt.AlignVCenter
      spacing: Theme.space(3)
      Label {
        Layout.fillWidth: true
        text: row.field.label || row.field.key || ""
      }
      Label {
        Layout.fillWidth: true
        visible: !!row.field.help
        role: "help"
        text: row.field.help || ""
      }
    }

    Loader {
      Layout.alignment: Qt.AlignVCenter | Qt.AlignRight
      active: !row.wide && row.control !== null
      sourceComponent: row.control
    }
  }

  Loader {
    Layout.fillWidth: true
    active: row.wide
    visible: active
    sourceComponent: row.control
  }

  Component {
    id: segmentedControl
    SegmentedControl { field: row.field; value: row.value; onEdited: function(v) { row.edit(v) } }
  }
  Component {
    id: cardsControl
    CardChoice { field: row.field; value: row.value; onEdited: function(v) { row.edit(v) } }
  }
  Component {
    id: chipsControl
    ChipChoice { field: row.field; value: row.value; onEdited: function(v) { row.edit(v) } }
  }
  Component {
    id: sliderControl
    SliderControl { field: row.field; value: row.value; onEdited: function(v) { row.edit(v) } }
  }
  Component {
    id: toggleControl
    Toggle { field: row.field; value: row.value; onEdited: function(v) { row.edit(v) } }
  }
  Component {
    id: textControl
    TextControl {
      field: row.field; value: row.value; stored: row.stored
      onEdited: function(v) { row.edit(v) }
    }
  }
  Component {
    id: modulesControl
    ModuleList {
      field: row.field; value: row.value; svc: row.svc
      onEdited: function(v) { row.edit(v) }
    }
  }
  Component {
    id: workspacesControl
    WorkspaceChips {
      field: row.field; value: row.value; svc: row.svc
      onEdited: function(v) { row.edit(v) }
    }
  }
  Component {
    id: notchesControl
    NotchCatalog {
      field: row.field; value: row.value; svc: row.svc
      onEdited: function(v) { row.edit(v) }
    }
  }
  Component {
    id: moduleCatalogControl
    ModuleCatalog { field: row.field; value: row.value; svc: row.svc }
  }
  Component {
    id: componentControl
    ComponentRow { field: row.field; value: row.value; svc: row.svc; onEdited: function(v) { row.edit(v) } }
  }
  Component {
    id: unknownControl
    Label {
      role: "help"
      wrapMode: Text.NoWrap
      text: "unsupported field type: " + row.field.type
      color: Theme.danger
    }
  }
}
