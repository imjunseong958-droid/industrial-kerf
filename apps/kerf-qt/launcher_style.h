#pragma once

#include "editor_theme.h"

#include <QColor>
#include <QString>

#include <utility>

namespace kerf {

// The project launcher follows the system appearance rather than the editor's
// dark finishing-room look: macOS's light and dark window colours, with the
// system accent for selection, switches, and the default button. Painted
// parts (tiles, sidebar symbols, switches) read the same tokens.
struct LauncherTheme {
    bool dark = false;
    QColor accent;
    QColor accentTop;
    QColor accentPressed;
    QColor onAccent;
    // Secondary text on `selection`: white held back only as far as it still
    // reads at 4.5:1 (all white where even white does not).
    QColor onAccentSecondary;
    QColor content;
    QColor titleText;
    QColor titleRule;
    QColor sidebar;
    QColor sidebarDivider;
    QColor text;
    // Secondary text, and the Details disclosure's chevron.
    QColor secondary;
    QColor disabledText;
    QColor separator;
    QColor field;
    QColor fieldBorder;
    QColor fieldDisabled;
    QColor focusRing;
    QColor textSelection;
    // A chosen row in a list that has focus, and in one that has not (the
    // sidebar's rows among them), with the detail text on the latter.
    QColor selection;
    QColor selectionInactive;
    QColor secondaryOnSelection;
    // Push buttons and pop-ups: a fill with a lighter top edge and a darker
    // bottom one, as macOS draws its bezels.
    QColor button;
    QColor buttonEdge;
    QColor buttonTop;
    QColor buttonBottom;
    QColor buttonPressed;
    // A disabled button is the same bezel at half strength.
    QColor buttonDisabled;
    QColor buttonDisabledEdge;
    QColor buttonDisabledText;
    QColor group;
    QColor groupBorder;
    QColor tile;
    QColor tileShadow;
    QColor tileGlyph;
    QColor badge;
    QColor switchOff;
    QColor switchOffStroke;
    QColor knob;
    QColor knobEdge;
    QColor error;
    QColor success;
    // A scroll bar's handle reads at 3:1 on the content, groups and sidebar
    // it sits on; under the pointer or held, more.
    QColor scrollHandle;
    QColor scrollHandleHover;
    QColor tooltip;
};

// `over` laid on `under` at `amount` (0 to 1).
inline QColor launcherMix(const QColor &under, const QColor &over, qreal amount) {
    return QColor::fromRgbF(under.redF() + (over.redF() - under.redF()) * amount,
                            under.greenF() + (over.greenF() - under.greenF()) * amount,
                            under.blueF() + (over.blueF() - under.blueF()) * amount);
}

inline LauncherTheme launcherTheme(bool dark, const QColor &accent) {
    LauncherTheme t;
    t.dark = dark;
    t.accent = accent;
    t.accentTop = accent.lighter(112);
    t.accentPressed = accent.darker(dark ? 120 : 115);
    t.onAccent = QColor("#ffffff");
    // White text needs a deeper blue than the accent's own, as in macOS's
    // selected rows.
    t.selection = launcherMix(accent, Qt::black, dark ? 0.16 : 0.12);
    t.onAccentSecondary = readableAlphaOn(QColor(255, 255, 255, 204), t.selection, 4.5);
    t.focusRing = accent;
    t.focusRing.setAlphaF(0.35);
    if (dark) {
        t.content = QColor("#1e1e1e");
        t.titleText = QColor("#dfdfdf");
        t.titleRule = QColor("#3a3a3c");
        t.sidebar = QColor("#2c2c2e");
        t.sidebarDivider = QColor("#3a3a3c");
        t.text = QColor("#f5f5f7");
        t.secondary = QColor("#98989d");
        t.disabledText = QColor("#6e6e73");
        t.separator = QColor("#38383a");
        t.field = QColor("#2c2c2e");
        t.fieldBorder = QColor("#48484a");
        t.fieldDisabled = QColor("#252527");
        t.selectionInactive = QColor("#3e3e41");
        t.secondaryOnSelection = QColor("#aeaeb2");
        t.button = QColor("#58585b");
        t.buttonEdge = QColor("#58585b");
        t.buttonTop = QColor("#6c6c70");
        t.buttonBottom = QColor("#4e4e51");
        t.buttonPressed = QColor("#6c6c70");
        t.buttonDisabled = QColor("#3b3b3d");
        t.buttonDisabledEdge = QColor("#444447");
        t.buttonDisabledText = QColor("#8a8a8e");
        t.group = QColor("#252527");
        t.groupBorder = QColor("#38383a");
        t.tile = QColor("#4a4c51");
        t.tileShadow = QColor(0, 0, 0, 115);
        t.badge = QColor("#6e7076");
        t.switchOff = QColor("#48484a");
        t.switchOffStroke = QColor("#48484a");
        t.knob = QColor("#e8e8ed");
        t.knobEdge = QColor(0, 0, 0, 60);
        t.error = QColor("#ff6961");
        t.success = QColor("#30d158");
        t.scrollHandle = QColor(255, 255, 255, 115);
        t.scrollHandleHover = QColor(255, 255, 255, 160);
        t.tooltip = QColor("#2c2c2e");
    } else {
        t.content = QColor("#ffffff");
        t.titleText = QColor("#262626");
        t.titleRule = QColor("#d4d4d4");
        t.sidebar = QColor("#f5f5f5");
        t.sidebarDivider = QColor("#e1e1e1");
        t.text = QColor("#1d1d1f");
        t.secondary = QColor("#6e6e73");
        t.disabledText = QColor("#a5a5aa");
        t.separator = QColor("#e3e3e6");
        t.field = QColor("#ffffff");
        t.fieldBorder = QColor("#c9c9ce");
        t.fieldDisabled = QColor("#f5f5f7");
        t.selectionInactive = QColor("#dcdcdf");
        t.secondaryOnSelection = QColor("#5f5f64");
        t.button = QColor("#ffffff");
        t.buttonEdge = QColor(0, 0, 0, 36);
        t.buttonTop = QColor(0, 0, 0, 28);
        t.buttonBottom = QColor(0, 0, 0, 72);
        t.buttonPressed = QColor("#ebebed");
        t.buttonDisabled = QColor("#ffffff");
        t.buttonDisabledEdge = QColor(0, 0, 0, 24);
        t.buttonDisabledText = QColor("#a5a5aa");
        t.group = QColor("#f6f6f8");
        t.groupBorder = QColor("#e3e3e6");
        t.tile = QColor("#5e6168");
        t.tileShadow = QColor(0, 0, 0, 46);
        t.badge = QColor("#8e9196");
        t.switchOff = QColor("#e9e9eb");
        t.switchOffStroke = QColor("#d1d1d6");
        t.knob = QColor("#ffffff");
        t.knobEdge = QColor(0, 0, 0, 38);
        t.error = QColor("#d70015");
        t.success = QColor("#1e7b34");
        t.scrollHandle = QColor(0, 0, 0, 115);
        t.scrollHandleHover = QColor(0, 0, 0, 160);
        t.tooltip = QColor("#f5f5f7");
    }
    t.tileGlyph = QColor("#ffffff");
    t.textSelection = launcherMix(t.field, accent, dark ? 0.45 : 0.28);
    return t;
}

// A complete style sheet for the launcher and everything in it. The editor's
// application style sheet stays in force for every property this one leaves
// out, so each widget type here sets all it relies on.
inline QString launcherStyleSheet(const LauncherTheme &t) {
    const auto c = [](const QColor &color) {
        return color.alpha() == 255 ? color.name() : color.name(QColor::HexArgb);
    };
    QString sheet = QStringLiteral(R"QSS(
* {
    color: @text;
    font-size: 13px;
}
QDialog#projectLauncher,
QWidget#launcherContent,
QMessageBox { background: @content; }
QWidget#launcherPagesArea,
QWidget#launcherPages,
QWidget#launcherHeadingBox,
QWidget#launcherDetails,
QWidget#launcherDetailsBody,
QWidget#presetSlots { background: transparent; border: 0; }
QToolTip {
    color: @text;
    background: @tooltip;
    border: 1px solid @fieldBorder;
    border-radius: 4px;
    padding: 4px 7px;
    font-size: 12px;
}

QLabel { background: transparent; border: 0; padding: 0; }
QLabel#launcherTitle {
    color: @titleText;
    background: @content;
    border: 0;
    border-bottom: 1px solid @titleRule;
    font-size: 13px;
    font-weight: 600;
}
QLabel#launcherHeading { font-size: 13px; font-weight: 600; }
QLabel#launcherSubheading { color: @secondary; font-size: 11px; }
QLabel#launcherHint { color: @secondary; font-size: 12px; }
QLabel#launcherEyebrow { color: @secondary; font-size: 11px; font-weight: 600; }
QLabel#launcherProblem { color: @secondary; font-size: 12px; }
QLabel#launcherProblem[state="error"] { color: @error; }
QLabel#launcherDetailsSummary { color: @secondary; font-size: 12px; }
QLabel#launcherFormLabel { padding-top: 3px; }
QLabel#presetSlotName { font-weight: 600; }
QLabel#presetStartLabel { color: @secondary; font-size: 12px; }
QLabel[fit="none"] { color: @secondary; font-size: 12px; }
QLabel[fit="yes"] { color: @success; font-size: 12px; }
QLabel[fit="no"] { color: @error; font-size: 12px; }

QListWidget,
QListView {
    background: transparent;
    border: 0;
    border-radius: 0;
    padding: 0;
    outline: 0;
    selection-background-color: transparent;
    selection-color: @text;
}
QFrame#launcherSidebarPane {
    background: @sidebar;
    border: 0;
    border-right: 1px solid @sidebarDivider;
}
QListWidget#launcherSidebar {
    background: transparent;
    border: 0;
    padding: 2px 8px 8px 8px;
}
QFrame#launcherSidebarFoot {
    background: transparent;
    border: 0;
    border-top: 1px solid @sidebarDivider;
}
QListWidget#presetChoices {
    background: @group;
    border: 1px solid @groupBorder;
    border-radius: 8px;
    padding: 0;
}
QFrame#presetSlotGroup {
    background: @group;
    border: 1px solid @groupBorder;
    border-radius: 8px;
}
QFrame#presetSlot { background: transparent; border: 0; }
QFrame#launcherGroupSeparator {
    background: @separator;
    border: 0;
    margin-left: 14px;
    min-height: 1px;
    max-height: 1px;
}
QWidget#launcherDetailsRow {
    background: transparent;
    border: 0;
    border-top: 1px solid @separator;
}
QFrame#launcherFooter {
    background: @content;
    border: 0;
    border-top: 1px solid @separator;
}

QScrollArea { background: transparent; border: 0; }
QScrollArea > QWidget > QWidget { background: transparent; }
QScrollBar:vertical,
QScrollBar:horizontal { background: transparent; border: 0; margin: 2px; }
QScrollBar:vertical { width: 10px; }
QScrollBar:horizontal { height: 10px; }
QScrollBar::handle:vertical,
QScrollBar::handle:horizontal { background: @scrollHandle; border: 0; border-radius: 3px; min-height: 28px; min-width: 28px; }
QScrollBar::handle:vertical:hover,
QScrollBar::handle:horizontal:hover,
QScrollBar::handle:vertical:pressed,
QScrollBar::handle:horizontal:pressed { background: @scrollHandleHover; }
QScrollBar::add-line,
QScrollBar::sub-line { width: 0; height: 0; border: 0; background: transparent; }
QScrollBar::add-page,
QScrollBar::sub-page { background: transparent; }

/* Its own height, not the editor's 22px button metrics. */
QToolButton#launcherDetailsToggle {
    background: transparent;
    border: 0;
    border-radius: 0;
    padding: 0;
    min-height: 19px;
    max-height: 19px;
    color: @text;
    font-size: 13px;
}

QPushButton,
QComboBox {
    background: @button;
    color: @text;
    border: 1px solid @buttonEdge;
    border-top-color: @buttonTop;
    border-bottom-color: @buttonBottom;
    border-radius: 5px;
    padding: 2px 12px;
    min-height: 16px;
    max-height: 16px;
    font-size: 13px;
    font-weight: 400;
    outline: 0;
}
QPushButton:pressed { background: @buttonPressed; }
QPushButton:disabled,
QComboBox:disabled {
    color: @buttonDisabledText;
    background: @buttonDisabled;
    border-color: @buttonDisabledEdge;
}
QPushButton#launcherChoose:enabled {
    background: qlineargradient(x1: 0, y1: 0, x2: 0, y2: 1, stop: 0 @accentTop, stop: 1 @accent);
    border-color: @accent;
    border-top-color: @accentTop;
    color: @onAccent;
}
QPushButton#launcherChoose { min-width: 58px; }
QPushButton#launcherChoose:pressed { background: @accentPressed; border-color: @accentPressed; }

QLineEdit,
QDoubleSpinBox {
    background: @field;
    color: @text;
    border: 1px solid @fieldBorder;
    border-radius: 5px;
    padding: 2px 7px;
    min-height: 16px;
    max-height: 16px;
    selection-background-color: @textSelection;
    selection-color: @text;
    /* Placeholders carry guidance, so they read at 4.5:1 as the editor's
       do; a disabled field's stay dimmed with the field. */
    placeholder-text-color: @secondary;
}
QLineEdit:focus,
QDoubleSpinBox:focus { border-color: @accent; }
QLineEdit:disabled,
QDoubleSpinBox:disabled { color: @disabledText; background: @fieldDisabled; border-color: @separator; placeholder-text-color: @disabledText; }
QDoubleSpinBox { padding-right: 2px; }
QAbstractSpinBox::up-button,
QAbstractSpinBox::down-button {
    subcontrol-origin: border;
    width: 16px;
    height: 11px;
    border: 0;
    background: transparent;
}
QAbstractSpinBox::up-button { subcontrol-position: top right; }
QAbstractSpinBox::down-button { subcontrol-position: bottom right; }
QAbstractSpinBox::up-arrow,
QAbstractSpinBox::down-arrow { image: none; border: 0; width: 0; height: 0; }

QComboBox { padding-left: 9px; padding-right: 24px; combobox-popup: 1; }
QComboBox::drop-down {
    subcontrol-origin: padding;
    subcontrol-position: center right;
    width: 22px;
    border: 0;
    background: transparent;
}
QComboBox::down-arrow { image: none; border: 0; width: 0; height: 0; }
QComboBox QAbstractItemView {
    background: @field;
    color: @text;
    border: 1px solid @fieldBorder;
    border-radius: 6px;
    padding: 4px;
    outline: 0;
    selection-background-color: @accent;
    selection-color: @onAccent;
}
QComboBox QAbstractItemView::item { min-height: 22px; padding: 0; border: 0; }
QCheckBox { background: transparent; spacing: 8px; }

QProgressBar {
    background: @separator;
    border: 0;
    border-radius: 2px;
    min-height: 4px;
    max-height: 4px;
}
QProgressBar::chunk { background: @accent; border-radius: 2px; }

QMenu {
    background: @field;
    border: 1px solid @fieldBorder;
    border-radius: 6px;
    padding: 4px;
}
QMenu::item { color: @text; background: transparent; padding: 3px 18px 3px 10px; border-radius: 4px; }
QMenu::item:selected { color: @onAccent; background: @accent; }
QMenu::item:disabled { color: @disabledText; background: transparent; }
QMenu::separator { height: 1px; background: @separator; margin: 4px 6px; }
)QSS");
    // Longer names first, so @accent does not eat @accentTop.
    const std::pair<const char *, QColor> tokens[] = {
        {"@accentPressed", t.accentPressed}, {"@accentTop", t.accentTop}, {"@onAccent", t.onAccent},
        {"@accent", t.accent}, {"@content", t.content}, {"@titleText", t.titleText}, {"@titleRule", t.titleRule},
        {"@sidebarDivider", t.sidebarDivider}, {"@sidebar", t.sidebar}, {"@textSelection", t.textSelection},
        {"@text", t.text}, {"@secondary", t.secondary}, {"@disabledText", t.disabledText},
        {"@separator", t.separator}, {"@fieldBorder", t.fieldBorder}, {"@fieldDisabled", t.fieldDisabled},
        {"@field", t.field}, {"@buttonEdge", t.buttonEdge}, {"@buttonTop", t.buttonTop},
        {"@buttonBottom", t.buttonBottom}, {"@buttonPressed", t.buttonPressed},
        {"@buttonDisabledEdge", t.buttonDisabledEdge}, {"@buttonDisabledText", t.buttonDisabledText},
        {"@buttonDisabled", t.buttonDisabled}, {"@button", t.button},
        {"@groupBorder", t.groupBorder}, {"@group", t.group}, {"@error", t.error}, {"@success", t.success},
        {"@scrollHandleHover", t.scrollHandleHover}, {"@scrollHandle", t.scrollHandle}, {"@tooltip", t.tooltip},
    };
    for (const auto &[token, color] : tokens) sheet.replace(QLatin1String(token), c(color));
    return sheet;
}

} // namespace kerf
