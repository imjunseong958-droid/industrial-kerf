#pragma once

#include "device_frame.h"
#include "editor_common.h"
#include "test_friends.h"

#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QMouseEvent>
#include <QPaintEvent>
#include <QPointer>
#include <QTransform>
#include <QVector>
#include <QWidget>

namespace kerf {

class PlayerOverlay;

// The Program monitor. It also draws the selected picture clip's masks over
// the picture and edits them: a mask tool draws a new one, a handle drag
// moves a corner or point, a drag inside a mask moves it, and Delete
// removes the selected one. Every change leaves as a typed operation.
class ViewerWidget final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    explicit ViewerWidget(QWidget *parent = nullptr);
    void setFrame(const QImage &frame, double time);
    QImage frame() const { return m_frame; }
    void clearFrame();
    void setLiveScrub(bool live);
    void setCaption(const QString &caption);
    void setProjectInfo(double width, double height);
    // The stage, as a Short's page lays out its player: the picture card,
    // the action rail on its right, and Kerf's conversation on the rail's
    // right, one group centred in the viewer. A portrait picture shows in a
    // phone (see DevicePreview) and a wide one in a laptop (see
    // LaptopPreview), whose body takes the card's place in the group. The
    // conversation is level with the card (or the device's body) and as
    // tall: where the stage has the room, the card is made large enough for
    // the conversation's least height (kMinChatHeight), the conversation
    // narrowing for it. With the conversation closed the card is centred
    // alone and the rail hangs on its right. Where the viewer is too narrow
    // for all three, the conversation narrows first, so the picture keeps
    // four fifths of its width (and a laptop's picture kLaptopKeeps of the
    // plain card's); past that the card shrinks, then the conversation lies
    // over the stage's right side.
    enum class Device { None, Phone, Laptop };
    struct StageLayout {
        QRectF card;   // The picture.
        QRectF body;   // The device it shows in; the card itself without one.
        QRectF screen; // The device's screen; empty without one.
        QRect rail;    // Empty when there is no room for it.
        QRect panel;   // Empty while the conversation is closed.
        bool overlay = false;
        bool device = false; // In a phone or a laptop.
        Device kind = Device::None;
    };
    // Round the card or the device: the same at the sides, over and under
    // it, for a card, a laptop and a phone alike (a device's shadow lies in
    // its frame's own pixmap, inside this).
    static constexpr int kStageMargin = 16;
    // Only a phone in a stage too short for its screen to be kLeastPhoneScreen
    // wide at kStageMargin over and under it takes less, as little as it
    // needs, never under kLeastStageInset, and still as much over as under.
    static constexpr int kLeastStageInset = 10;
    static constexpr double kLeastPhoneScreen = 240.0;
    static constexpr int kStageGap = 12;
    // Between card, rail and conversation in a narrow stage.
    static constexpr int kNarrowStageGap = 8;
    static constexpr int kNarrowStage = 1000;
    static constexpr int kCardRadius = 12;
    static constexpr int kRailWidth = 48;
    // The conversation's width: kept between these (a per-user preference).
    static constexpr int kMinChatWidth = 300;
    static constexpr int kMaxChatWidth = 480;
    static constexpr int kDefaultChatWidth = 432;
    // The card (or device) is made tall enough for the conversation beside
    // it to be this tall where the stage has the room, so its header, a
    // couple of comments, the next steps' chips and its composer fit...
    static constexpr int kMinChatHeight = 352;
    // ...and where it has not (the conversation at its narrowest beside a
    // wide picture), the conversation is still as tall as the card down to
    // this, then no shorter (the card centred on it).
    static constexpr int kLeastChatHeight = 280;
    // The card's narrowest before the conversation narrows, then lies over it.
    static constexpr int kMinCardWidth = 160;
    // The share of its width with the conversation closed that the card
    // keeps before the conversation gives up any of its own.
    static constexpr double kCardKeeps = 0.8;
    // The share of the plain card's width (in the same stage, with the
    // conversation open) a laptop's picture keeps: its bezel and base come
    // mostly out of the conversation's width.
    static constexpr double kLaptopKeeps = 0.92;
    // The rail and conversation, laid out in the stage as children.
    void setStage(QWidget *rail, QWidget *panel);
    void setChatOpen(bool open);
    bool chatOpen() const { return m_chatOpen; }
    void setChatWidth(int width);
    int chatWidth() const { return m_chatWidth; }
    // The phone a portrait picture shows in, or Off for the plain card, and
    // the outlines of where a vertical-video app covers the picture.
    void setDevicePreview(DevicePreview preview);
    DevicePreview devicePreview() const { return m_device; }
    void setSafeZones(bool shown);
    bool safeZonesShown() const { return m_safeZones; }
    // The laptop a wide picture shows in, or Off for the plain card.
    void setLaptopPreview(LaptopPreview preview);
    LaptopPreview laptopPreview() const { return m_laptop; }
    // Whether the picture is taller than wide, or wider than tall.
    bool portrait() const { return aspect() < 1.0; }
    // The player's compact arrangement: on a portrait card, and on a wide
    // one too narrow for the row of controls and the time over the length.
    static constexpr double kCompactPlayerWidth = 360.0;
    bool compactPlayer(const QRectF &card) const { return portrait() || card.width() < kCompactPlayerWidth; }
    bool landscape() const { return aspect() > 1.0; }
    // The device the picture shows in, as the previews and its shape choose.
    Device device() const;
    // How many times a device's frame was drawn rather than blitted from
    // its cache (see DeviceFrameCache).
    int deviceFrameRenders() const { return m_frameCache.renders(); }
    int stageGap() const;
    // The room over (and as much under) the card or the device.
    int stageInset(Device device) const;
    StageLayout stageLayout() const { return stageLayout(device()); }
    // The stage as it would be with the picture in `device`.
    StageLayout stageLayout(Device device) const;
    QRectF videoRect() const;
    // Where the caption preview sits: near the picture's foot, lifted over
    // the player's controls while they show, as a video site lifts its
    // captions, gliding with the controls' fade.
    QRectF captionRect() const;
    PlayerOverlay *overlay() const { return m_overlay; }

    // The masks of picture clip `clip` as its snapshot carries them, and
    // where its picture lands on the canvas (picturePlacement). An empty
    // clip shows none and stops a mask tool.
    void setMasks(const QString &clip, const QJsonArray &masks, const QTransform &placement);
    void setMaskTool(MaskTool tool);
    MaskTool maskTool() const { return m_maskTool; }
    void selectMask(const QString &name);
    QString selectedMask() const { return m_selectedMask; }

signals:
    void scrubbed(double fraction);
    void scrubEnded();
    void editRequested(const QJsonArray &ops);
    // The conversation's width changed (a drag on its edge).
    void chatWidthChanged(int width);
    // A mask was drawn or the tool was put down.
    void maskToolChanged(MaskTool tool);
    // A click chose a mask, or none.
    void maskSelected(const QString &name);

protected:
    bool event(QEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;
    void leaveEvent(QEvent *) override;
    void paintEvent(QPaintEvent *) override;
    void resizeEvent(QResizeEvent *) override;

private:
    // Place the rail and the conversation where stageLayout() puts them.
    void layoutStage();
    double aspect() const;
    // Across the picture, as the player's bar is.
    double fractionAt(double x) const;
    // While a tool is out or the clip has masks, the pointer edits masks
    // instead of scrubbing or seeking.
    bool editsMasks() const;
    // Fractions of the clip's picture to widget pixels.
    QTransform pictureToWidget() const;
    QPointF picturePoint(const QPointF &widget) const;
    int maskIndex(const QString &name) const;
    // The selected mask's handle under the pointer, or -1.
    int handleAt(const QPointF &widget) const;
    // The topmost mask under the pointer, the selected one first.
    QString maskAt(const QPointF &widget) const;
    void updateCursor(const QPointF &widget);
    void paintMasks(QPainter &p);
    // Add `mask` (an outline) as the next free `k1`, `k2`, …, select it, and
    // put the tool down.
    void addMask(QJsonObject mask);
    void finishPen(bool closed);
    void cancelDrawing();
    void removeSelectedMask();

    QImage m_frame;
    QImage m_scaled;
    double m_time = 0.0;
    double m_width = 0.0;
    double m_height = 0.0;
    QPointer<QWidget> m_rail;
    QPointer<QWidget> m_panel;
    bool m_chatOpen = false;
    int m_chatWidth = kDefaultChatWidth;
    DevicePreview m_device = DevicePreview::Island;
    LaptopPreview m_laptop = LaptopPreview::Laptop;
    DeviceFrameCache m_frameCache;
    bool m_safeZones = false;
    PlayerOverlay *m_overlay = nullptr;
    double m_skimFraction = -1.0;
    bool m_skimming = false;
    bool m_liveScrub = false;
    bool m_scaledFast = false;
    QString m_caption;

    QString m_maskClip;
    QJsonArray m_masks;
    QTransform m_placement;
    MaskTool m_maskTool = MaskTool::None;
    QString m_selectedMask;
    // A rectangle or ellipse being dragged out, in picture fractions.
    bool m_drawing = false;
    QPointF m_drawStart;
    QPointF m_drawEnd;
    // The pen's points so far, in picture fractions, and the pointer.
    QVector<QPointF> m_penPoints;
    QPointF m_pointer;
    // A handle (its index), the whole mask (kMoveMask), or nothing being
    // dragged, from where and from which outline.
    static constexpr int kMoveMask = -2;
    int m_dragHandle = -1;
    QPointF m_dragFrom;
    QJsonObject m_dragOriginal;
};

class RegionPicker final : public QWidget {
public:
    explicit RegionPicker(const QImage &frame, const QRectF &region, QWidget *parent = nullptr);
    QRectF region() const { return m_region; }

protected:
    void paintEvent(QPaintEvent *) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;

private:
    QRectF imageRect() const;
    QPointF normalizedPoint(const QPointF &point) const;

    QImage m_frame;
    QRectF m_region;
    QPointF m_start;
    bool m_dragging = false;
};

} // namespace kerf
