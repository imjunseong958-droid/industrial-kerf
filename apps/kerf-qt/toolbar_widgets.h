#pragma once

#include "test_friends.h"

#include <QAbstractButton>
#include <QList>
#include <QPointer>
#include <QSize>
#include <QString>
#include <QToolBar>
#include <QWidget>

class QAction;
class QActionGroup;
class QMenu;

namespace kerf {

// The main toolbar as the editor's faceplate: a plate lit from above, its
// 1px catch of light along the top and its edge along the foot, in both
// appearances. Its controls sit on it as they would on any toolbar.
class FaceplateToolBar final : public QToolBar {
    Q_OBJECT
public:
    explicit FaceplateToolBar(const QString &title, QWidget *parent = nullptr);

protected:
    void paintEvent(QPaintEvent *event) override;
};

// One key of the format bank: the format's shape outlined on a raised key,
// with no words. Held in (and its LED lit) while it is the project's
// format. A click takes no keyboard focus, so the transport keys keep
// working; Tab reaches it, ←→ move along the bank and Space or Return
// chooses.
class FormatKey final : public QAbstractButton {
    Q_OBJECT
public:
    static constexpr int kWidth = 28;
    static constexpr int kHeight = 20;

    FormatKey(const QString &ratio, const QSize &format, QWidget *parent = nullptr);
    QSize format() const { return m_format; }
    // The outlined shape, as it is drawn on the key (for tests).
    QRectF shapeRect() const;
    QSize sizeHint() const override;
    // Whether Tab or an arrow brought focus here, so it is ringed.
    bool showsFocusRing() const { return hasFocus() && m_ring; }

signals:
    // ← (-1) or → (+1) asked to move focus along the bank.
    void stepRequested(int step);

protected:
    void paintEvent(QPaintEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;
    void focusInEvent(QFocusEvent *event) override;
    void focusOutEvent(QFocusEvent *event) override;
    // The bank latches keys; a click only asks.
    void nextCheckState() override {}

private:
    QSize m_format;
    bool m_ring = false;
};

// The project's format as a bank of latching keys: an engraved FORMAT and
// one key per canvas preset, the current one held in. A size no preset has
// latches none, and the bank's tooltip names it ("Custom · 1280 × 720").
class FormatBank final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    static constexpr int kHeight = FormatKey::kHeight + 4;

    explicit FormatBank(QWidget *parent = nullptr);
    // Latch the key for `size`, or none; it does not emit.
    void setCanvasSize(const QSize &size);
    QSize canvasSize() const { return m_size; }
    // The latched key's index, or -1 for a custom size.
    int currentIndex() const;
    int count() const { return int(m_keys.size()); }
    FormatKey *key(int index) const { return m_keys.value(index); }
    // The recessed bank holding the keys, in the widget's coordinates.
    QRectF bankRect() const;
    QSize sizeHint() const override;
    QSize minimumSizeHint() const override { return sizeHint(); }

signals:
    // A key was chosen; it is latched already, and the size is the preset's.
    void formatChosen(const QSize &size);

protected:
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    void changeEvent(QEvent *event) override;

private:
    int labelWidth() const;
    void layoutKeys();
    void choose(FormatKey *key);

    QList<FormatKey *> m_keys;
    QSize m_size;
};

// Export as one split key in the accent: the main part exports, a chip in
// it names the quality (MASTER or DRAFT), and the arrow part opens the
// menu: Master or Draft, and Render Preview. Master, the finished file, is
// the default.
class ExportButton final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    static constexpr int kHeight = 20;
    static constexpr int kArrowWidth = 18;

    // `exportAction` and `renderAction` are the commands' own actions: the
    // main part follows the first's enablement and tooltip, and the menu
    // shows the second with its own.
    ExportButton(QAction *exportAction, QAction *renderAction, QWidget *parent = nullptr);
    // As file.export takes it: "full" or "preview".
    QString quality() const;
    // As people read it: "Master" or "Draft".
    QString qualityName() const;
    void setQuality(const QString &quality);
    // What Master makes, beside it in the menu ("1920 × 1080 · 24 fps").
    void setMasterDetail(const QString &detail);

    QAbstractButton *mainPart() const { return m_main; }
    QAbstractButton *menuPart() const { return m_arrow; }
    QMenu *menu() const { return m_menu; }
    QAction *masterAction() const { return m_master; }
    QAction *draftAction() const { return m_draft; }
    // The chip naming the quality, in the widget's coordinates.
    QRectF chipRect() const;
    QSize sizeHint() const override;
    QSize minimumSizeHint() const override { return sizeHint(); }

    // Open the menu under the button (the arrow part, or ↓ on either part).
    void showMenu();

signals:
    void qualityChanged(const QString &quality);
    // The menu is about to open, for its details to be brought up to date.
    void menuAboutToShow();

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    void changeEvent(QEvent *event) override;

private:
    QFont labelFont() const;
    QFont chipFont() const;
    void syncFromAction();

    QPointer<QAction> m_exportAction;
    QAbstractButton *m_main = nullptr;
    QAbstractButton *m_arrow = nullptr;
    QMenu *m_menu = nullptr;
    QActionGroup *m_qualities = nullptr;
    QAction *m_master = nullptr;
    QAction *m_draft = nullptr;
    bool m_menuOpen = false;
};

} // namespace kerf
