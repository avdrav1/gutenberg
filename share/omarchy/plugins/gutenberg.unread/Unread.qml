import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons as Commons
import qs.Commons
import qs.Ui

// Gutenberg's unread stories on the bar: a newspaper and the count, from
// `gutenberg --unread` every 30 seconds. A click opens Gutenberg, a right
// click counts again. install.sh writes the binary's path in for
// @GUTENBERG@.
BarWidget {
  id: root
  moduleName: "gutenberg.unread"

  readonly property string binary: "@GUTENBERG@"
  // -1 until the first count, or when the command fails.
  property int unread: -1

  function refresh() {
    if (!countProc.running) countProc.running = true
  }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  Commons.Translations { id: strings; directory: Qt.resolvedUrl("i18n") }

  Process {
    id: countProc
    command: [root.binary, "--unread"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        // Only a plain number is taken; anything else reads as no count.
        var raw = String(text || "").trim()
        root.unread = /^[0-9]{1,9}$/.test(raw) ? parseInt(raw, 10) : -1
      }
    }
    onExited: function(exitCode) {
      if (exitCode !== 0) root.unread = -1
    }
  }

  Timer {
    interval: 30000
    running: true
    repeat: true
    triggeredOnStart: true
    onTriggered: root.refresh()
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: ""
    labelText: root.unread > 0 && !vertical ? Commons.I18n.formatNumber(root.unread) : ""
    slotSize: Style.bar.iconSlot * (root.unread > 0 && !vertical ? 2 : 1)
    tooltipText: root.unread > 0
      ? strings.tn("tooltip.unread", root.unread).arg(Commons.I18n.formatNumber(root.unread))
      : root.unread === 0 ? strings.t("tooltip.none") : strings.t("tooltip.missing")
    onPressed: function(b) {
      if (b === Qt.RightButton) root.refresh()
      else Util.execArgv([root.binary])
    }
  }
}
