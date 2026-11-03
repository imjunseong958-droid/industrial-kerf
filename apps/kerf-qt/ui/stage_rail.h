#pragma once

// The stage rail beside the device (mockup-spec 3.5): 44 wide and, at the
// design's size, 548 tall. From the top:
//
//   - The program meter (top 8, 44 x 350): two 6-wide bars at x 6 and 14,
//     Y(dB) = -dB/60 of its height, run track2 to -18, warn to -6, neg to 0,
//     with a 2 pt peak-hold line, and the dB scale on its right. It is fed
//     only by the bridge's `meter` events (ProgramMeter); with none it shows
//     its empty track.
//   - L and R under the bars (top 362), then the momentary loudness
//     (top 390): "−17.8" over "LUFS M", "—" with no reading. Clicking the
//     meter or the readout runs workspace.finish and asks the bus to
//     highlight the loudness pane.
//   - The device button (32 x 32 at 6, 438) with its sub-chevron. It opens
//     the Device menu to its right (Device preview · <format>: Laptop or the
//     phone's choices, and Off) and emits devicePreviewRequested; the stage
//     sets the preview. Its label (Laptop, Phone, Feed or Off) is under it.
//   - The Comments button (6, 498), on while the panel is open, with a badge
//     of the review's open threads; "Comments" under it.
//
// A rail shorter or taller than 548 keeps the lower group at its foot and
// gives the meter what is left (350 at 548); a meter under 60 tall is hidden.
// The two labels are 60 wide, wider than the rail, so they are drawn by small
// label widgets beside it in its parent.
//
// The rail hears the engine through the EditorBus (meter events, the review's
// counts, the program's frame size). Given none, it finds its window's bus
// (the window's "editorBus" child) once it is placed in one; and it reads the
// shortcuts of the window's registry actions window.assistant and
// view.devicePreview for its tips, unless setActions() gave them.

#include "components.h"

#include <QPointer>
#include <QString>
#include <QWidget>

#include <functional>

class QAction;
class QEnterEvent;
class QMenu;

namespace kerf {

class EditorBus;
class ProgramMeter;
class VuMeter;

// A rail button (.rbtn): 32 x 32, radius 6, a 16 pt icon in ink2. Hover:
// raised and ink; on: the on fill, ink and an inset line2. The device button
// carries a 10 pt chevron (rail-chevron, ink3) at its lower right.
class RailButton final : public ui::KitButton {
    Q_OBJECT
public:
    // The comments, the phone a portrait picture shows in, the laptop a wide
    // one shows in, and safe zones (retired from the rail; see StageRail).
    enum class Glyph { Chat, Phone, Laptop, SafeZones };
    static constexpr int kDisc = 32;
    explicit RailButton(Glyph glyph, QWidget *parent = nullptr);
    Glyph glyph() const { return m_glyph; }
    void setGlyph(Glyph glyph);
    // The icon drawn: set by the rail for the device (g-laptop, g-phone,
    // g-feed, g-devoff); otherwise the glyph's.
    void setIconName(const QString &icon);
    QString iconName() const;
    void setChevron(bool chevron);
    bool hasChevron() const { return m_chevron; }
    // Drawn on while its menu is open.
    void setHeld(bool held);
    // Whether it is drawn on now: checked (unless its owner sets its checked
    // state, as the device's says whether the device shows), or held.
    bool drawsOn() const;
    // Deprecated, removed in W3: the rail draws its own labels under its
    // buttons; this only keeps what callers set.
    void setLabel(const QString &label) { m_label = label; }
    QString label() const { return m_label; }
    // The button's square, in its own coordinates (the whole button).
    QRect discRect() const { return rect(); }
    QSize sizeHint() const override { return {kDisc, kDisc}; }
    // A button whose on or off comes from its owner, not from a click: the
    // device button opens a menu instead.
    void setCheckedByOwner(bool owned) { m_checkedByOwner = owned; }
    // A retired button is never shown, whatever its old owner asks.
    void setRetired(bool retired);
    bool retired() const { return m_retired; }
    void setVisible(bool visible) override;
    // Called whenever its glyph or checked state changes, even with its
    // signals blocked (its owner's syncs block them).
    void setStateCallback(std::function<void()> callback) { m_stateCallback = std::move(callback); }

protected:
    void paintEvent(QPaintEvent *) override;
    void nextCheckState() override {
        if (!m_checkedByOwner) QAbstractButton::nextCheckState();
    }
    void checkStateSet() override;
    bool event(QEvent *event) override;

private:
    Glyph m_glyph;
    QString m_icon;
    QString m_label;
    bool m_chevron = false;
    bool m_held = false;
    bool m_checkedByOwner = false;
    bool m_retired = false;
    std::function<void()> m_stateCallback;
};

class StageRail final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kWidth = 44;
    // The design's height (TOP 8 to the scrub's foot).
    static constexpr int kHeight = 548;
    static constexpr int kMeterTop = 8;
    // The meter's height at kHeight.
    static constexpr int kMeterHeight = 350;
    // Under this the meter (and its L/R) is hidden.
    static constexpr int kMeterMinHeight = 60;
    // Deprecated, removed in W3 (the old meter's gap over the buttons).
    static constexpr int kMeterGap = 12;
    // The labels under the buttons: 60 wide, centred on the rail.
    static constexpr int kLabelWidth = 60;

    explicit StageRail(QWidget *parent = nullptr, EditorBus *bus = nullptr);
    ~StageRail() override;

    void setBus(EditorBus *bus);
    EditorBus *bus() const;
    // The registry actions whose shortcuts the tips show: the comments
    // panel's switch (window.assistant) and the device preview's.
    void setActions(QAction *comments, QAction *device);

    RailButton *chatButton() const { return m_chat; }
    RailButton *deviceButton() const { return m_device; }
    // Deprecated, removed in W3: safe zones moved to the Program strip; this
    // button is retired and never shown.
    RailButton *safeZonesButton() const { return m_safeZones; }
    // Deprecated, removed in W3: the old VU meter, kept (hidden) so its old
    // callers compile. The meter is programMeter() and levelMeter().
    VuMeter *meter() const { return m_vu; }

    ProgramMeter *programMeter() const { return m_meter; }
    QWidget *levelMeter() const { return m_level; }
    QWidget *lufsReadout() const { return m_lufs; }
    ui::Badge *commentsBadge() const { return m_badge; }
    QWidget *deviceLabelWidget() const;
    QWidget *commentsLabelWidget() const;

    // Geometry, in the rail's coordinates.
    int meterHeight() const;
    QRect meterRect() const;
    // Channel 0 (L) at x 6, 1 (R) at x 14: 6 wide, as tall as the meter.
    QRect barRect(int channel) const;
    // Where `db` sits on the bars, from the meter's top.
    double meterY(double db) const;
    QRect channelLabelRect(int channel) const;
    // The scale label for `mark` dB under full scale: x 24, 20 wide,
    // right-aligned, its 16 pt line centred on the mark.
    QRect scaleLabelRect(int mark) const;
    // The marks shown at this height (those 12 pt clear of the one above).
    QList<int> shownScaleMarks() const;
    QRect lufsRect() const;
    QRect deviceRect() const;
    QRect deviceLabelRect() const;
    QRect commentsRect() const;
    QRect commentsLabelRect() const;

    // "−17.8" or "—".
    QString lufsText() const;

    // ---- the device -------------------------------------------------------
    // The picture's shape and whether its device shows. Until this is
    // called, the rail reads the shape from the bus (the program's frame)
    // and whether it shows from the device button's checked state.
    void setDevicePreview(double pictureAspect, bool on);
    double deviceAspect() const;
    bool deviceOn() const;
    // A wide picture shows in the laptop, any other in the phone.
    bool deviceIsLaptop() const;
    // "16:9", "9:16", "1:1", "4:5"...
    QString formatName() const;
    // "Laptop", "Phone", "Feed" or "Off".
    QString deviceLabel() const;
    // "Device preview · 16:9".
    QString deviceMenuHeader() const;
    // The Device menu's items: on (the device) or off, the name, its glyph,
    // and whether it is the one in use.
    struct DeviceChoice {
        bool on = false;
        QString name;
        QString icon;
        bool checked = false;
    };
    QList<DeviceChoice> deviceChoices() const;
    // The Device menu, styled by the kit; choosing an item emits
    // devicePreviewRequested. The caller owns it.
    QMenu *createDeviceMenu(QWidget *parent = nullptr);
    // Opens it to the right of the device button.
    void showDeviceMenu();

    // ---- the comments -----------------------------------------------------
    // The review's open threads, or -1 with no review.
    int openCount() const;
    // "Comments · 4 open", or "Comments" with no review.
    QString commentsTip() const;

signals:
    // The Device menu chose the device (true) or the plain card (false).
    void devicePreviewRequested(bool on);
    // The meter or the readout was clicked (after workspace.finish ran and
    // the bus was asked to highlight the loudness pane).
    void loudnessRequested();

protected:
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    void moveEvent(QMoveEvent *event) override;
    bool event(QEvent *event) override;

private:
    friend class LevelMeterWidget;
    friend class LufsReadoutWidget;
    void requestLoudness();
    void attachToWindow();
    void relayout();
    void placeLabels();
    void watchReview();
    void refreshComments();
    void refreshDevice();
    void refreshTips();
    QString shortcutOf(const QAction *action) const;

    QPointer<EditorBus> m_bus;
    QMetaObject::Connection m_reviewChanged;
    QList<QMetaObject::Connection> m_busConnections;
    QList<QMetaObject::Connection> m_reviewConnections;
    QPointer<QAction> m_commentsAction;
    QPointer<QAction> m_deviceAction;
    bool m_actionsGiven = false;
    ProgramMeter *m_meter = nullptr;
    QWidget *m_level = nullptr;
    QWidget *m_lufs = nullptr;
    RailButton *m_device = nullptr;
    RailButton *m_safeZones = nullptr;
    RailButton *m_chat = nullptr;
    ui::Badge *m_badge = nullptr;
    VuMeter *m_vu = nullptr;
    QPointer<QWidget> m_deviceLabel;
    QPointer<QWidget> m_commentsLabel;
    bool m_hasDevice = false;
    double m_deviceAspect = 16.0 / 9.0;
    bool m_deviceOn = true;
};

} // namespace kerf
