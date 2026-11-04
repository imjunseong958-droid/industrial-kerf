#pragma once

#include "design_tokens.h"
#include "device_frame.h"
#include "editor_common.h"
#include "test_friends.h"

#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QList>
#include <QMouseEvent>
#include <QPaintEvent>
#include <QPointer>
#include <QTransform>
#include <QVector>
#include <QWidget>

class QGraphicsOpacityEffect;
class QVariantAnimation;

namespace kerf {

class PlayerOverlay;

// The Program stage (mockup-spec 3): the picture in its device, the rail on
// the device's right, the comments panel anchored to the stage's right edge,
// the scrub strip under the device, the agent HUD and, in the Edit workspace
// and the phone formats, the Source column on the left. It also draws the
// picture's overlays (safe zones, the caption, the QC overlap box and the
// comments' drawings), captures a drawing on the frame, and draws the
// selected picture clip's masks over the picture and edits them: a mask tool
// draws a new one, a handle drag moves a corner or point, a drag inside a
// mask moves it, and Delete removes the selected one. Every change leaves as
// a typed operation.
//
// Placement (the [OWNER] table of mockup-spec 3.1): the device's top is
// kStageTop; the scrub strip's (SY) is the stage's height less 64; the
// laptop's box is SY - 9 - TOP tall and the phone's SY - 12 - TOP. The panel
// keeps a 12 margin to the stage's right edge, the rail stands 12 right of
// the device. With the panel open a 16:9 device and its rail are centred in
// 16 ... panel - 12, and a phone stands 12 left of the rail; closed, the rail
// ends 12 from the stage's edge and the Source column fills the left. The
// device is scaled to fit its box at its product's exact proportions, its
// outer rect snapped to whole points; "Off" draws the picture as a plain
// rect where the device's picture would be.
class ViewerWidget final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    explicit ViewerWidget(QWidget *parent = nullptr);
    void setFrame(const QImage &frame, double time);
    QImage frame() const { return m_frame; }
    void clearFrame();
    void setLiveScrub(bool live);
    // The engine's caption at the playhead (a `caption {text}` event).
    void setCaption(const QString &caption);
    QString caption() const { return m_caption; }
    void setProjectInfo(double width, double height);

    enum class Device { None, Phone, Laptop };
    struct StageLayout {
        QRectF card;    // The picture (at Fit).
        QRectF body;    // The device; the picture itself when the device is Off.
        QRectF screen;  // The device's screen; empty without one.
        QRect rail;     // The rail's geometry (railRect rounded).
        QRectF railRect;
        QRect panel;    // The comments panel; empty while it is closed.
        QRect panelSlot; // Where the panel stands when open (its width for this format).
        QRectF scrub;   // The scrub strip under the device.
        bool narrowScrub = false;
        QRect source;   // The Source column; empty when it does not show.
        QRectF hud;     // The agent HUD: a card in title safe, or a strip over the Source column.
        bool hudStrip = false;
        bool phone = false;   // A phone format (9:16, 1:1, 4:5): the phone, its 480 panel.
        bool overlay = false; // Too narrow for the group: the panel lies over the stage.
        bool device = false;  // In a phone or a laptop.
        Device kind = Device::None;
    };

    // ---- placement (tokens::layout) ----
    static constexpr int kStageTop = tokens::layout::stageTop;          // the device's top
    static constexpr int kStageFoot = tokens::layout::stageBottom;      // under the scrub strip
    static constexpr int kScrubHeight = tokens::layout::scrub;          // 56
    static constexpr int kLaptopScrubGap = 9;                            // laptop base to the scrub
    static constexpr int kPhoneScrubGap = 12;                            // phone body to the scrub
    static constexpr int kStageEdge = tokens::layout::stageMargin;      // 12: rail and panel to the edge, device to rail
    static constexpr int kSourceInset = tokens::layout::stageInset;     // 16: the Source column's (and the group's) left
    static constexpr int kRailWidth = tokens::layout::rail;             // 44
    static constexpr int kPhoneScrubWidth = 360;
    static constexpr int kPhoneSourceWidth = 400;
    // The Source column's least width; narrower, it does not show.
    static constexpr int kLeastSourceWidth = 240;
    // The device's least width before the panel lies over the stage.
    static constexpr int kMinDeviceWidth = 160;
    // A 16:9 picture narrower than this has the scrub strip's narrow mode.
    static constexpr int kNarrowScrubWidth = 480;
    static constexpr int kHudWidth = 288;
    static constexpr int kHudHeight = 72;
    static constexpr int kHudStripHeight = 32;
    // The panel's width: 368 beside a 16:9 picture, 480 beside a phone,
    // dragged between 300 and 480 and remembered per format.
    static constexpr int kMinChatWidth = tokens::layout::panelMin;
    static constexpr int kMaxChatWidth = tokens::layout::panelMax;
    static constexpr int kDefaultChatWidth = tokens::layout::panel;
    static constexpr int kPhoneChatWidth = tokens::layout::panelPhone;
    // Stage and device moves, the panel's open and close.
    static constexpr int kMoveDuration = tokens::motion::move;

    // The rail and the comments panel, laid out in the stage as children.
    void setStage(QWidget *rail, QWidget *panel);
    // The scrub strip, the agent HUD and the Source column, the same way.
    // The HUD shows and hides itself; the stage follows.
    void setStageParts(QWidget *scrub, QWidget *hud, QWidget *source);
    QWidget *scrubStrip() const { return m_scrub; }
    QWidget *hud() const { return m_hud; }
    QWidget *sourceColumn() const { return m_source; }
    void setChatOpen(bool open);
    bool chatOpen() const { return m_chatOpen; }
    // The panel's width for the format in use (300-480).
    void setChatWidth(int width);
    int chatWidth() const { return phoneFormat() ? m_phoneChatWidth : m_chatWidth; }
    // The width for 16:9 (`phone` false) or the phone formats.
    void setChatWidth(bool phone, int width);
    int chatWidth(bool phone) const { return phone ? m_phoneChatWidth : m_chatWidth; }

    // The phone a portrait, square or 4:5 picture shows in, or Off.
    void setDevicePreview(DevicePreview preview);
    DevicePreview devicePreview() const { return m_device; }
    // Action and title safe over the picture, every format.
    void setSafeZones(bool shown);
    bool safeZonesShown() const { return m_safeZones; }
    // The laptop a wide picture shows in, or Off.
    void setLaptopPreview(LaptopPreview preview);
    LaptopPreview laptopPreview() const { return m_laptop; }
    // Whether the picture is taller than wide, or wider than tall.
    bool portrait() const { return aspect() < 1.0; }
    bool landscape() const { return aspect() > 1.0; }
    // The picture's width over its height.
    double pictureAspect() const { return aspect(); }
    // A phone format: 9:16, 1:1, 4:5 (anything up to square).
    bool phoneFormat() const { return aspect() <= 1.0 + 1e-3; }
    // The device the picture shows in, as the previews and its shape choose.
    Device device() const;
    // How many times a device's frame was drawn rather than blitted from
    // its cache (see DeviceFrameCache).
    int deviceFrameRenders() const { return m_frameCache.renders(); }
    // The stage as it is laid out (where a move is going to).
    StageLayout stageLayout() const { return stageLayout(device()); }
    // The stage with the picture in `device` (None: the device Off).
    StageLayout stageLayout(Device device) const;
    // The stage as it is drawn now, part way through a move.
    StageLayout shownLayout() const;
    QRectF videoRect() const;
    // The picture as drawn: the card at Fit, else the frame at the zoom,
    // centred on the card (clipped to the screen).
    QRectF pictureRect() const;
    // Where the caption sits: centred, its foot on title safe's (mockup-spec 3.3).
    QRectF captionRect() const;
    PlayerOverlay *overlay() const { return m_overlay; }
    // Where the agent HUD's pop shadow reaches (empty while it is hidden).
    QRectF hudShadowRect() const;

    // Dim surround: the stage's backdrop from surround to surroundDim over
    // 220 ms (view.dim, and the Finish workspace).
    void setDim(bool dim);
    bool dimmed() const { return m_dim; }
    QColor surroundColor() const;
    // The comments' drawings over the picture (view.annotations).
    void setAnnotationsShown(bool shown);
    bool annotationsShown() const { return m_annotationsShown; }
    // One comment's drawing to show now: strokes normalised to the frame
    // ({tool: pen|ellipse|rect|arrow, points: [[x, y], ...], color}) and
    // the initials on the 16 pt avatar at the first stroke's start.
    struct Annotation {
        QString comment;
        QJsonArray drawing;
        QString initials;
    };
    void setAnnotations(const QList<Annotation> &annotations);
    QList<Annotation> annotations() const { return m_annotations; }
    // The QC overlap box: `rect` normalised to the frame, `tag` its text
    // ("8 px"); an empty rect hides it (fading over 160 ms).
    void setOverlap(const QRectF &rect, const QString &tag);
    QRectF overlap() const { return m_overlapRect; }
    QString overlapTag() const { return m_overlapTag; }
    bool overlapShown() const { return m_overlapShown; }
    // The overlap box and its tag in widget points, as drawn.
    QRectF overlapBox() const;
    QRectF overlapTagRect() const;

    // Viewer zoom: 0 is Fit, else the share of the frame's own pixels (0.5,
    // 1, 2), one frame pixel a device pixel.
    void setZoom(double zoom);
    double zoom() const { return m_zoom; }

    // A drawing on the frame (the composer's Draw): the pointer draws pen
    // strokes over the picture; Return finishes (drawingFinished with the
    // strokes, normalised to the frame), Esc cancels.
    void beginDrawing();
    void finishDrawing();
    void cancelDrawingCapture();
    bool capturingDrawing() const { return m_capturing; }
    QJsonArray capturedStrokes() const;

    // The masks of picture clip `clip` as its snapshot carries them, and
    // where its picture lands on the canvas (picturePlacement). An empty
    // clip shows none and stops a mask tool.
    void setMasks(const QString &clip, const QJsonArray &masks, const QTransform &placement);
    void setMaskTool(MaskTool tool);
    MaskTool maskTool() const { return m_maskTool; }
    void selectMask(const QString &name);
    QString selectedMask() const { return m_selectedMask; }

    // Tests: -1 moves where something shows them (not on the offscreen
    // platform, not under reduced motion), 0 never, 1 always.
    static void setMotionOverride(int state);
    bool moving() const;

signals:
    void scrubbed(double fraction);
    void scrubEnded();
    void editRequested(const QJsonArray &ops);
    // The panel's width changed (a drag on its edge).
    void chatWidthChanged(int width);
    // A mask was drawn or the tool was put down.
    void maskToolChanged(MaskTool tool);
    // A click chose a mask, or none.
    void maskSelected(const QString &name);
    void drawingFinished(const QJsonArray &strokes);
    void drawingCancelled();
    // Placed again (a resize, the panel, the format): the stage's parts moved.
    void stageLaidOut();

protected:
    bool event(QEvent *event) override;
    bool eventFilter(QObject *watched, QEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;
    void leaveEvent(QEvent *) override;
    void paintEvent(QPaintEvent *) override;
    void resizeEvent(QResizeEvent *) override;
    void showEvent(QShowEvent *) override;

private:
    // Place the stage's parts where the layout puts them now (part way
    // through a move while one runs: see setChatOpen).
    void layoutStage();
    void applyLayout(const StageLayout &layout);
    // The panel's fade while it opens or closes, `m_move` of the way; none
    // once the move ends.
    void fadePanel();
    // Ends a move under way at once (a format or device change jumps).
    void jump();
    bool motionAllowed() const;
    double aspect() const;
    // Across the picture, as the player's bar is.
    double fractionAt(double x) const;
    // While a tool is out or the clip has masks, the pointer edits masks
    // instead of scrubbing or seeking.
    bool editsMasks() const;
    // Fractions of the clip's picture to widget pixels.
    QTransform pictureToWidget() const;
    QPointF picturePoint(const QPointF &widget) const;
    // Widget points to fractions of the frame, and back.
    QPointF frameFraction(const QPointF &widget) const;
    QPointF fromFrame(const QPointF &fraction) const;
    int maskIndex(const QString &name) const;
    // The selected mask's handle under the pointer, or -1.
    int handleAt(const QPointF &widget) const;
    // The topmost mask under the pointer, the selected one first.
    QString maskAt(const QPointF &widget) const;
    void updateCursor(const QPointF &widget);
    void paintMasks(QPainter &p);
    void paintCaption(QPainter &p, bool flagged);
    void paintOverlap(QPainter &p);
    void paintAnnotations(QPainter &p);
    void paintStrokes(QPainter &p, const QJsonArray &drawing, const QString &initials);
    // The caption's text box: its lines wrapped (and balanced) on a phone,
    // one line on 16:9, and the box with its padding.
    QRectF captionBox(QString *text) const;
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
    QPointer<QWidget> m_scrub;
    QPointer<QWidget> m_hud;
    QPointer<QWidget> m_source;
    bool m_chatOpen = false;
    int m_chatWidth = kDefaultChatWidth;
    int m_phoneChatWidth = kPhoneChatWidth;
    DevicePreview m_device = DevicePreview::Phone;
    LaptopPreview m_laptop = LaptopPreview::Laptop;
    DeviceFrameCache m_frameCache;
    bool m_safeZones = true;
    PlayerOverlay *m_overlay = nullptr;
    double m_skimFraction = -1.0;
    bool m_skimming = false;
    bool m_liveScrub = false;
    bool m_scaledFast = false;
    QString m_caption;

    // The move under way: from the layout drawn when it began to the
    // layout's target, `m_move` (eased) of the way.
    QVariantAnimation *m_mover = nullptr;
    StageLayout m_from;
    double m_move = 1.0;
    bool m_laidOut = false;
    QPointer<QGraphicsOpacityEffect> m_panelFade;
    // Dim, 0 (surround) to 1 (surroundDim).
    bool m_dim = false;
    double m_dimLevel = 0.0;
    QVariantAnimation *m_dimmer = nullptr;
    bool m_annotationsShown = true;
    QList<Annotation> m_annotations;
    QRectF m_overlapRect;
    QString m_overlapTag;
    bool m_overlapShown = false;
    double m_overlapLevel = 0.0;
    QVariantAnimation *m_overlapFade = nullptr;
    double m_zoom = 0.0;
    // The drawing being captured: finished strokes and the one in hand, in
    // fractions of the frame.
    bool m_capturing = false;
    QList<QVector<QPointF>> m_strokes;
    QVector<QPointF> m_stroke;

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
