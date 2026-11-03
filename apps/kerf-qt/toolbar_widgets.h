#pragma once

// The toolbar's and the status bar's flat controls (mockup-spec 1, 9, 14),
// painted from design_tokens.h and the kit (ui/components.h, ui/icons.h) so
// the app's style sheet cannot change them:
//
//   FlatToolBar      the 40 pt toolbar: chrome with a bottom hairline; its
//                    content is one row laid over it
//   ToolbarSegments  the toolbar's .seg (28 tall): text or icon segments, a
//                    sliding fill, an optional menu tail, and no segment on
//                    when nothing it offers is current (a custom frame size,
//                    one's own workspace)
//   FormatBank       the format segments 16:9 / 9:16 / 1:1 / 4:5
//   ExportButton     the Export split button: "Export  Master" and a chevron
//   StatusRow        the 24 pt status bar's row, laid over the QStatusBar
//   StatusText       one status item: UI text with an optional mono tail
//   StatusProgress   "Draft export", a 64 x 4 bar and the percentage
//   StatusChip       a 20 pt chip (the keymap chip, the sync chip)
//   AgentStatus      "Agent: own key", a 6 pt dot and the connection
//
// The hardware faceplate (FaceplateToolBar, FormatKey and the accent
// ExportButton) is gone; FormatBank and ExportButton keep their names and
// the API MainWindow uses (W3 may rename them).

#include "design_tokens.h"
#include "test_friends.h"
#include "ui/components.h"

#include <QAbstractButton>
#include <QColor>
#include <QDateTime>
#include <QList>
#include <QPointer>
#include <QSize>
#include <QString>
#include <QToolBar>
#include <QToolButton>
#include <QVariant>
#include <QWidget>

class QAction;
class QActionGroup;
class QHBoxLayout;
class QMenu;
class QVariantAnimation;

namespace kerf {

// ---------------------------------------------------------------- helpers

// "25p", "23.98p", "29.97p": a frame rate as the toolbar and status bar
// name it.
QString frameRateLabel(double fps);
// "1920×1080 · 25p": the sequence button's text.
QString sequenceLabel(const QSize &size, double fps);
// "just now", "1 min ago", "5 min ago", "2 h ago", "yesterday", "3 days
// ago", else the date ("4 Nov 2026"): how long ago `then` was at `now`.
QString editedAgo(const QDateTime &then, const QDateTime &now);
// The design's name for a format preset ("Landscape 16:9", "Vertical
// 9:16", "Square 1:1", "Portrait 4:5"), and its tip with the size
// ("Landscape 16:9 · 1920×1080").
QString formatName(int presetIndex);
QString formatTip(int presetIndex);

// ---------------------------------------------------------------- FlatToolBar

// The window's toolbar: chrome with an inset 1 pt line at its foot, 40 tall.
// It holds no actions; setContent() lays one widget over its whole rect.
class FlatToolBar final : public QToolBar {
    Q_OBJECT
public:
    explicit FlatToolBar(const QString &title, QWidget *parent = nullptr);
    void setContent(QWidget *content);
    QWidget *content() const { return m_content; }

protected:
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;

private:
    QWidget *m_content = nullptr;
};
// Deprecated: the faceplate is gone; removed in W3.
using FaceplateToolBar = FlatToolBar;

// ---------------------------------------------------------------- ToolbarSegments

// .seg in the toolbar (mockup-spec 0.5): a raised track 28 tall with 2 pt
// padding and radius 6; segments 24 tall, radius 4. Text segments are pad
// 0 8, UI 12/16 ink2, hover ink, on ink 500, each as wide as its label at
// weight 500. Icon segments are `iconWidth` wide (28 in the toolbar). The
// sliding fill (on, inset line2, a 6% top highlight) moves in 200 ms. An
// optional 20 pt tail with a 14 pt chevron asks for a menu. Current -1
// shows no segment on.
class ToolbarSegments : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    static constexpr int kHeight = 28;
    static constexpr int kTailWidth = 20;

    explicit ToolbarSegments(QWidget *parent = nullptr);

    int addSegment(const QString &text, const QString &tip = {}, const QString &shortcut = {});
    int addIconSegment(const QString &icon, const QString &tip = {}, const QString &shortcut = {});
    int count() const { return int(m_segments.size()); }
    QAbstractButton *segment(int index) const { return m_segments.value(index); }
    QString segmentText(int index) const;
    void setSegmentTip(int index, const QString &tip, const QString &shortcut = {});
    void setIconWidth(int width);
    void setMenuTail(const QString &tip, const QString &shortcut = {});
    QAbstractButton *menuTail() const { return m_tail; }

    int currentIndex() const { return m_current; }
    // -1 turns every segment off. Emits nothing.
    void setCurrentIndex(int index, bool animate = true);
    QRectF segmentRect(int index) const;
    // The fill as drawn now (empty while no segment is on).
    QRectF fillRect() const { return m_current >= 0 ? m_fill : QRectF(); }

    QSize sizeHint() const override;
    QSize minimumSizeHint() const override { return sizeHint(); }

signals:
    // A segment was clicked or chosen with ←→ (even the current one).
    void activated(int index);
    void menuRequested();

protected:
    void paintEvent(QPaintEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;
    void focusInEvent(QFocusEvent *event) override;
    void focusOutEvent(QFocusEvent *event) override;
    void changeEvent(QEvent *event) override;

private:
    int add(const QString &text, const QString &icon, const QString &tip, const QString &shortcut);
    void relayout();

    QList<ui::KitButton *> m_segments;
    ui::KitButton *m_tail = nullptr;
    QVariantAnimation *m_slide = nullptr;
    QRectF m_fill;
    int m_current = -1;
    int m_iconWidth = 28;
    bool m_focusVisible = false;
};

// ---------------------------------------------------------------- FormatBank

// The format segments (icon w28): 16:9 (⌥1), 9:16 (⌥2), 1:1 (⌥3), 4:5 (⌥4),
// one per canvas preset, each tipped "Landscape 16:9 · 1920×1080". The
// project's size turns its preset on; a size no preset has turns none on
// and the bank's tooltip names it ("Custom · 1280 × 720").
class FormatBank final : public ToolbarSegments {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    explicit FormatBank(QWidget *parent = nullptr);
    // Turns on the preset for `size`, or none; it does not emit.
    void setCanvasSize(const QSize &size);
    QSize canvasSize() const { return m_size; }
    QSize format(int index) const;
    // The preset index of `size`, or -1.
    static int presetIndex(const QSize &size);
    // Deprecated (the bank's keys are segments now), removed in W3.
    QAbstractButton *key(int index) const { return segment(index); }

signals:
    // A segment was chosen; it is on already, and the size is its preset's.
    void formatChosen(const QSize &size);

private:
    QSize m_size;
};

// ---------------------------------------------------------------- ExportButton

// The Export split button (mockup-spec 1): 28 tall, radius 6, an inset 1 pt
// line2 outline. The main part reads "Export" (UI 500 12 ink) and the
// quality (" Master", ink3 400), pad 0 10, gap 6, hover raised; it exports
// that quality. The 24 pt chevron part, set off by an inset line, opens the
// menu. The menu's first rows are Master (ProRes 422 HQ) and Draft (H.264
// review), an exclusive pair; the wiring adds the rest each time it opens.
class ExportButton final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    static constexpr int kHeight = 28;
    static constexpr int kArrowWidth = 24;

    explicit ExportButton(QWidget *parent = nullptr);
    // As the bridge's export takes it: "master" or "review".
    QString exportQuality() const;
    // As File ▸ Export… (file.export) takes it: "full" or "preview".
    // Deprecated (MainWindow::exportProject), removed in W3.
    QString quality() const;
    // As people read it: "Master" or "Draft".
    QString qualityName() const;
    // The codec the quality renders with: "prores_hq" or "h264".
    QString codec() const;
    // "master"/"full" or "review"/"preview".
    void setQuality(const QString &quality);

    QAbstractButton *mainPart() const { return m_main; }
    QAbstractButton *menuPart() const { return m_arrow; }
    QMenu *menu() const { return m_menu; }
    QAction *masterAction() const { return m_master; }
    QAction *draftAction() const { return m_draft; }
    // Enables the main part, with the tip it shows ("Export master" and
    // its key, or why it cannot export).
    void setExportEnabled(bool enabled, const QString &tip, const QString &shortcut = {});
    QSize sizeHint() const override;
    QSize minimumSizeHint() const override { return sizeHint(); }

    // Opens the menu under the chevron (menuAboutToShow first).
    void showMenu();

signals:
    // The main part was clicked: export `quality` ("master" or "review").
    void exportRequested(const QString &quality);
    // The chosen quality changed ("master" or "review").
    void qualityChanged(const QString &quality);
    // The menu is about to open, for its rows to be built.
    void menuAboutToShow();

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    void relayout();

    ui::KitButton *m_main = nullptr;
    ui::KitButton *m_arrow = nullptr;
    QMenu *m_menu = nullptr;
    QActionGroup *m_qualities = nullptr;
    QAction *m_master = nullptr;
    QAction *m_draft = nullptr;
    QString m_quality = QStringLiteral("master");
};

// ---------------------------------------------------------------- status bar

// The status bar's row (mockup-spec 9): chrome with an inset 1 pt line at
// its top, pad 0 12 0 16, gap 16, laid over the QStatusBar so the bar's own
// items and style never show. Its left part gives way to the status bar's
// message while one shows.
class StatusRow final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    explicit StatusRow(QWidget *parent = nullptr);
    QHBoxLayout *leftLayout() const { return m_left; }
    QHBoxLayout *rightLayout() const { return m_right; }
    // The status bar's current message; empty shows the left items again.
    void setMessage(const QString &message);
    QString message() const;
    QWidget *leftItems() const { return m_leftItems; }

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QWidget *m_leftItems = nullptr;
    class StatusText *m_message = nullptr;
    QHBoxLayout *m_left = nullptr;
    QHBoxLayout *m_right = nullptr;
};

// One status item: `text` in UI 11/16, then `monoTail` in mono 11/16, in
// one colour; elided at the right when narrower than its text.
class StatusText final : public QWidget {
    Q_OBJECT
public:
    explicit StatusText(QWidget *parent = nullptr);
    void setText(const QString &text, const QString &monoTail = {});
    QString text() const { return m_text; }
    QString monoTail() const { return m_mono; }
    QString fullText() const { return m_text + m_mono; }
    void setColor(QRgb color);
    QColor color() const { return tokens::color(m_color); }
    // The widget may shrink below its text (the selection gives way first).
    void setElidable(bool elidable);
    QSize sizeHint() const override;
    QSize minimumSizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QString m_text;
    QString m_mono;
    QRgb m_color = tokens::ink2;
    bool m_elidable = false;
};

// "Draft export", a 64 x 4 bar (track, ink2 fill) and the mono percentage,
// gap 8, ink3 label and ink2 number.
class StatusProgress final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kBarWidth = 64;
    explicit StatusProgress(QWidget *parent = nullptr);
    void setProgress(const QString &label, double fraction);
    QString label() const { return m_label; }
    double fraction() const { return m_fraction; }
    QString percentText() const;
    QRectF barRect() const;
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QString m_label;
    double m_fraction = 0;
};

// The status bar's chips (.gb): 20 tall, pad 0 6, radius 6, gap 4, an
// optional 14 pt leading icon, UI or mono 11/16 in ink2 (or the state's
// colour); hover raised + ink. A QToolButton so MainWindow keeps its type.
class StatusChip final : public QToolButton {
    Q_OBJECT
public:
    static constexpr int kHeight = 20;
    explicit StatusChip(QWidget *parent = nullptr);
    void setIconName(const QString &icon);
    QString iconName() const { return m_icon; }
    void setMono(bool mono);
    // The text's colour at rest (ink2 by default).
    void setInk(QRgb ink);
    QColor ink() const;
    QSize sizeHint() const override;
    QSize minimumSizeHint() const override { return sizeHint(); }

protected:
    void paintEvent(QPaintEvent *event) override;
    void enterEvent(QEnterEvent *event) override;
    void leaveEvent(QEvent *event) override;

private:
    QString m_icon;
    QRgb m_ink = tokens::ink2;
    bool m_mono = false;
};

// "Agent: own key", a 6 pt dot and "Connected", gap 6, UI 11/16 ink2.
class AgentStatus final : public QWidget {
    Q_OBJECT
public:
    explicit AgentStatus(QWidget *parent = nullptr);
    void setStatus(const QString &agent, QRgb dot, const QString &connection);
    QString agentText() const { return m_agent; }
    QString connectionText() const { return m_connection; }
    QColor dotColor() const { return tokens::color(m_dot); }
    QRectF dotRect() const;
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QString m_agent;
    QString m_connection;
    QRgb m_dot = tokens::dis;
};

} // namespace kerf
