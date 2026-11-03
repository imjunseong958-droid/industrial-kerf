#pragma once

// The action rail beside the picture: its round buttons and the program's
// VU meter. Moved verbatim from chat_panel.h; Q-SCRUBRAIL restyles it in W2.

#include <QAbstractButton>
#include <QString>
#include <QWidget>

class QEnterEvent;

namespace kerf {

class VuMeter;

// A round button on the action rail between the picture and the chat, as
// beside a Short: a disc with a symbol, and a count or label under it. The
// disc is a raised key of the hardware finish; on, it is held in and its
// amber LED is lit at its top.
class RailButton final : public QAbstractButton {
    Q_OBJECT
public:
    // The chat, the phone a portrait picture shows in, the laptop a wide
    // one shows in, and safe zones.
    enum class Glyph { Chat, Phone, Laptop, SafeZones };
    static constexpr int kDisc = 44;
    explicit RailButton(Glyph glyph, QWidget *parent = nullptr);
    Glyph glyph() const { return m_glyph; }
    void setGlyph(Glyph glyph);
    void setLabel(const QString &label);
    QString label() const { return m_label; }
    // The disc, in the button's coordinates.
    QRect discRect() const;
    QSize sizeHint() const override;
    // A button whose on or off comes from its owner, not from a click: the
    // device button opens a menu, and showing the opposite state while the
    // menu is open said the preview had already changed.
    void setCheckedByOwner(bool owned) { m_checkedByOwner = owned; }

protected:
    void paintEvent(QPaintEvent *) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }
    void nextCheckState() override {
        if (!m_checkedByOwner) QAbstractButton::nextCheckState();
    }

private:
    Glyph m_glyph;
    QString m_label;
    bool m_checkedByOwner = false;
};

// The rail's buttons, stacked at its foot, level with the picture's bottom
// edge, as Shorts stacks its actions: the device (a phone for a portrait
// picture, a laptop for a wide one) and safe zones (portrait only), then
// the chat. At its head, level with the picture's top edge, the program's
// VU meter, as wide as the rail; a rail too short for both keeps its
// buttons.
class StageRail final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kWidth = 48;
    static constexpr int kMeterHeight = 40;
    // The least room between the meter and the first button.
    static constexpr int kMeterGap = 12;
    explicit StageRail(QWidget *parent = nullptr);
    RailButton *chatButton() const { return m_chat; }
    RailButton *deviceButton() const { return m_device; }
    RailButton *safeZonesButton() const { return m_safeZones; }
    VuMeter *meter() const { return m_meter; }

protected:
    // A resize, or a button shown or hidden, refits the meter.
    bool event(QEvent *event) override;

private:
    void fitMeter();
    VuMeter *m_meter = nullptr;
    RailButton *m_device = nullptr;
    RailButton *m_safeZones = nullptr;
    RailButton *m_chat = nullptr;
};

} // namespace kerf
