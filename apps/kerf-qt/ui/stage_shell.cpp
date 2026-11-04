#include "stage_shell.h"

#include "design_tokens.h"

#include <QAbstractSpinBox>
#include <QApplication>
#include <QEvent>
#include <QKeyEvent>
#include <QLineEdit>
#include <QPainter>
#include <QPlainTextEdit>
#include <QResizeEvent>
#include <QTextEdit>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace stage {

QRect fitted(const QRect &slot, double aspect) {
    if (slot.isEmpty() || aspect <= 0.0) return slot;
    int width = slot.width();
    int height = int(std::lround(width / aspect));
    if (height > slot.height()) {
        height = slot.height();
        width = std::min(slot.width(), int(std::lround(height * aspect)));
    }
    return QRect(slot.left() + (slot.width() - width) / 2, slot.top() + (slot.height() - height) / 2, width, height);
}

Rects rects(const QSize &window, double aspect, int toolbar) {
    using namespace tokens::layout;
    Rects out;
    const double shape = aspect > 0.0 ? aspect : 16.0 / 9.0;
    const int w = std::max(0, window.width());
    const int h = std::max(0, window.height() - toolbar);
    out.wide = isWide(shape);
    if (out.wide) {
        // The conversation gives way only when the picture would go under
        // 600 wide, down to 300.
        const int conversation = std::min(w, std::clamp(w - leastPicture, conversationLeast, conversationWide));
        const int width = w - conversation;
        // The picture's height follows its aspect while the timeline keeps its least.
        const int most = std::max(0, h - timelineLeast);
        const int slotHeight = std::min(most, int(std::lround(width / shape)));
        out.slot = QRect(0, toolbar, width, slotHeight);
        out.conversation = QRect(width, toolbar, conversation, slotHeight);
        out.timeline = QRect(0, toolbar + slotHeight, w, h - slotHeight);
    } else {
        // The picture at the content's height; the right side keeps its least.
        const int right = std::max(std::min(w, rightLeast), w - int(std::lround(h * shape)));
        const int width = w - right;
        const int timeline = std::min(h, std::clamp(h - conversationLeast, timelineLeast, band));
        out.slot = QRect(0, toolbar, width, h);
        out.conversation = QRect(width, toolbar, right, h - timeline);
        out.timeline = QRect(width, toolbar + h - timeline, right, timeline);
    }
    out.picture = fitted(out.slot, shape);
    const auto sheet = [&](int width) {
        const int shown = std::min(width, w);
        return QRect(w - shown, out.conversation.top(), shown, out.conversation.height());
    };
    out.media = sheet(bin);
    out.browser = sheet(browser);
    out.inspector = sheet(inspector);
    const int tray = std::min(h, band);
    out.tray = QRect(0, toolbar + h - tray, w, tray);
    return out;
}

} // namespace stage

namespace {

constexpr int kSeam = tokens::layout::seam;

// Painted, not filled from the palette: the app sheet's `QWidget { background:
// transparent }` would leave a palette fill invisible.
class Seam final : public QWidget {
public:
    Seam(QWidget *parent, const char *name) : QWidget(parent) {
        setObjectName(QString::fromLatin1(name));
        setAttribute(Qt::WA_TransparentForMouseEvents);
    }

protected:
    void paintEvent(QPaintEvent *event) override {
        QPainter p(this);
        p.fillRect(event->rect(), tokens::color(tokens::line));
    }
};

QWidget *seam(QWidget *parent, const char *name) { return new Seam(parent, name); }

} // namespace

StageShell::StageShell(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("stageShell"));
    setAttribute(Qt::WA_OpaquePaintEvent);
    m_seamUpright = seam(this, "stageSeamUpright");
    m_seamAcross = seam(this, "stageSeamAcross");
    m_seamSheet = seam(this, "stageSeamSheet");
    m_seamTray = seam(this, "stageSeamTray");
    m_seamSheet->hide();
    m_seamTray->hide();
}

void StageShell::setRegions(QWidget *picture, QWidget *conversation, QWidget *timeline) {
    m_picture = picture;
    m_conversation = conversation;
    m_timeline = timeline;
    for (QWidget *region : {picture, conversation, timeline})
        if (region) {
            region->setParent(this);
            region->show();
        }
    place();
}

void StageShell::setSheets(QWidget *media, QWidget *browser, QWidget *inspector) {
    m_sheets[0] = media;
    m_sheets[1] = browser;
    m_sheets[2] = inspector;
    for (QWidget *sheet : m_sheets)
        if (sheet) {
            sheet->setParent(this);
            sheet->hide();
        }
    place();
}

void StageShell::setTrays(QWidget *finish, QWidget *deliver) {
    m_trays[0] = finish;
    m_trays[1] = deliver;
    for (QWidget *tray : m_trays)
        if (tray) {
            tray->setParent(this);
            tray->hide();
        }
    place();
}

void StageShell::setBanners(const QList<QWidget *> &banners) {
    m_banners = banners;
    for (QWidget *banner : banners) {
        banner->setParent(this);
        // Shown or hidden, or its text rewrapped, the bars stack again.
        banner->installEventFilter(this);
    }
    place();
}

QWidget *StageShell::widgetOf(Sheet sheet) const { return sheet == Sheet::None ? nullptr : m_sheets[int(sheet) - 1]; }

QWidget *StageShell::widgetOf(Tray tray) const { return tray == Tray::None ? nullptr : m_trays[int(tray) - 1]; }

QString StageShell::sheetId(Sheet sheet) {
    switch (sheet) {
    case Sheet::Media: return QStringLiteral("media");
    case Sheet::Browser: return QStringLiteral("browser");
    case Sheet::Inspector: return QStringLiteral("inspector");
    case Sheet::None: break;
    }
    return {};
}

void StageShell::setAspect(double aspect) {
    if (aspect <= 0.0 || aspect == m_aspect) return;
    m_aspect = aspect;
    place();
}

stage::Rects StageShell::rects() const {
    constexpr int toolbar = tokens::layout::toolbar;
    stage::Rects out = stage::rects(QSize(width(), height() + toolbar), m_aspect, toolbar);
    for (QRect *rect : {&out.slot, &out.picture, &out.conversation, &out.timeline, &out.media, &out.browser, &out.inspector, &out.tray})
        rect->translate(0, -toolbar);
    return out;
}

void StageShell::setSheet(Sheet sheet) {
    if (sheet == m_sheet) return;
    m_sheet = sheet;
    place();
    followEscape();
    emit sheetChanged(sheet);
}

void StageShell::toggleSheet(Sheet sheet) { setSheet(m_sheet == sheet ? Sheet::None : sheet); }

void StageShell::setTray(Tray tray) {
    if (tray == m_tray) return;
    m_tray = tray;
    place();
    followEscape();
    emit trayChanged(tray);
}

bool StageShell::dismiss() {
    if (m_sheet != Sheet::None) {
        setSheet(Sheet::None);
        return true;
    }
    if (m_tray != Tray::None) {
        setTray(Tray::None);
        return true;
    }
    return false;
}

void StageShell::followEscape() {
    const bool want = m_sheet != Sheet::None || m_tray != Tray::None;
    if (want == m_filtering || !qApp) return;
    m_filtering = want;
    if (want) qApp->installEventFilter(this);
    else qApp->removeEventFilter(this);
}

bool StageShell::takesEscape(QObject *watched, QEvent *event) {
    if (event->type() != QEvent::ShortcutOverride && event->type() != QEvent::KeyPress) return false;
    auto *key = static_cast<QKeyEvent *>(event);
    if (key->key() != Qt::Key_Escape || key->modifiers() != Qt::NoModifier) return false;
    auto *target = qobject_cast<QWidget *>(watched);
    if (!target || target->window() != window() || QApplication::activePopupWidget() || QApplication::activeModalWidget()) return false;
    // A text field and the picture (a drawing, a mask tool) answer first; what
    // they leave reaches the window, which closes it too.
    if (qobject_cast<QLineEdit *>(target) || qobject_cast<QTextEdit *>(target) || qobject_cast<QPlainTextEdit *>(target) ||
        qobject_cast<QAbstractSpinBox *>(target) || target == m_picture)
        return false;
    if (event->type() == QEvent::ShortcutOverride) {
        event->accept();
        return true;
    }
    dismiss();
    return true;
}

bool StageShell::holdsOpen(const QWidget *widget) const {
    for (const QWidget *open : {widgetOf(m_sheet), widgetOf(m_tray)})
        if (open && (open == widget || open->isAncestorOf(widget))) return true;
    return false;
}

void StageShell::place() {
    const stage::Rects r = rects();
    if (m_picture) m_picture->setGeometry(r.slot);
    if (m_conversation) m_conversation->setGeometry(r.conversation);
    if (m_timeline) m_timeline->setGeometry(r.timeline);
    // One hairline between the regions, drawn inside the conversation's and
    // the timeline's edges.
    if (r.wide) {
        m_seamUpright->setGeometry(r.conversation.left(), r.conversation.top(), kSeam, r.conversation.height());
        m_seamAcross->setGeometry(r.timeline.left(), r.timeline.top(), r.timeline.width(), kSeam);
    } else {
        m_seamUpright->setGeometry(r.conversation.left(), r.conversation.top(), kSeam, r.conversation.height() + r.timeline.height());
        m_seamAcross->setGeometry(r.timeline.left(), r.timeline.top(), r.timeline.width(), kSeam);
    }
    m_seamUpright->raise();
    m_seamAcross->raise();

    // The tray, over the regions; a sheet over the tray.
    for (int index = 0; index < 2; ++index) {
        QWidget *tray = m_trays[index];
        if (!tray) continue;
        const bool shown = int(m_tray) == index + 1;
        if (shown) {
            tray->setGeometry(r.tray);
            tray->raise();
        }
        tray->setVisible(shown);
    }
    m_seamTray->setVisible(m_tray != Tray::None);
    if (m_tray != Tray::None) {
        m_seamTray->setGeometry(r.tray.left(), r.tray.top(), r.tray.width(), kSeam);
        m_seamTray->raise();
    }
    const QRect sheetRects[3] = {r.media, r.browser, r.inspector};
    for (int index = 0; index < 3; ++index) {
        QWidget *sheet = m_sheets[index];
        if (!sheet) continue;
        const bool shown = int(m_sheet) == index + 1;
        if (shown) {
            sheet->setGeometry(sheetRects[index]);
            sheet->raise();
        }
        sheet->setVisible(shown);
    }
    m_seamSheet->setVisible(m_sheet != Sheet::None);
    if (m_sheet != Sheet::None) {
        const QRect sheet = sheetRects[int(m_sheet) - 1];
        m_seamSheet->setGeometry(sheet.left() - kSeam, sheet.top(), kSeam, sheet.height());
        m_seamSheet->raise();
    }
    // The bars, stacked at the top of the content, over everything.
    int top = 0;
    for (QWidget *banner : std::as_const(m_banners)) {
        if (banner->isHidden()) continue;
        const int height = banner->hasHeightForWidth() ? banner->heightForWidth(width()) : banner->sizeHint().height();
        banner->setGeometry(0, top, width(), height);
        banner->raise();
        top += height;
    }
}

void StageShell::resizeEvent(QResizeEvent *) { place(); }

void StageShell::paintEvent(QPaintEvent *event) {
    // Nothing shows through: the regions fill the shell.
    QPainter p(this);
    p.fillRect(event->rect(), tokens::color(tokens::chrome));
}

bool StageShell::eventFilter(QObject *watched, QEvent *event) {
    if (watched->isWidgetType() && m_banners.contains(static_cast<QWidget *>(watched)) &&
        (event->type() == QEvent::ShowToParent || event->type() == QEvent::HideToParent || event->type() == QEvent::LayoutRequest))
        place();
    if (takesEscape(watched, event)) return true;
    return QWidget::eventFilter(watched, event);
}

} // namespace kerf
