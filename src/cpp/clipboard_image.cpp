#include "clipboard_image.h"

#include <QBuffer>
#include <QByteArray>
#include <QClipboard>
#include <QGuiApplication>
#include <QImage>
#include <qqml.h>

ClipboardImage::ClipboardImage(QObject *parent)
    : QObject(parent)
{
}

QString ClipboardImage::pngBase64() const
{
    const QImage image = QGuiApplication::clipboard()->image();
    if (image.isNull())
        return QString();

    QByteArray bytes;
    QBuffer buffer(&bytes);
    buffer.open(QIODevice::WriteOnly);
    image.save(&buffer, "PNG");

    return QString::fromLatin1(bytes.toBase64());
}

extern "C" void booklet_register_clipboard_image()
{
    qmlRegisterType<ClipboardImage>("booklet", 1, 0, "ClipboardImage");
}
